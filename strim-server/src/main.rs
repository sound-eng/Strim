use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::time::Duration;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;

use strim_shared::{Message, AudioConfig, SampleFormat as SharedSampleFormat};

mod cli_commands;
use clap::Parser;

fn main() {
    let args = cli_commands::Cli::parse();
    
    // Set up graceful shutdown handling
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);
    let running_accept = Arc::clone(&running);
    let running_broadcast = Arc::clone(&running);
    let running_health = Arc::clone(&running);

    let ip_address = get_local_ip().unwrap_or_else(|| "unknown".parse().unwrap());
    println!("Server IP address: {}", ip_address);

    
    ctrlc::set_handler(move || {
        println!("\nReceived Ctrl+C, shutting down server gracefully...");
        running_clone.store(false, Ordering::SeqCst);
    }).expect("Failed to set Ctrl+C handler");
    
    let (tx, rx) = mpsc::channel::<Message>();

    let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
    let clients_for_accept = Arc::clone(&clients);
    let clients_for_config = Arc::clone(&clients);
    let clients_for_shutdown = Arc::clone(&clients);
    let clients_for_health = Arc::clone(&clients);
    
    // Get audio config first
    let audio_config = match get_audio_config() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("Error getting audio config: {err}");
            return;
        }
    };
    
    // Spawn a new clients accept thread
    thread::spawn(move || accept_loop(clients_for_accept, audio_config, args.port, running_accept));

    // Spawn a thread thet broadcasts recorded data to all clients
    thread::spawn(move || broadcast_loop(rx, clients_for_config, running_broadcast));
    
    // Spawn a health check thread to detect disconnected clients
    thread::spawn(move || health_check_loop(clients_for_health, running_health));

    let _stream = match start_default_input_capture(tx) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("Error starting capture: {err}");
            return;
        }
    };

    println!("Server running.");
    
    // Keep the main thread alive until shutdown signal
    while running.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(100));
    }
    
    // Graceful shutdown: notify all clients
    println!("Notifying clients of server shutdown...");
    let shutdown_msg = Message::Error("Server is shutting down".to_string());
    if let Ok(serialized) = shutdown_msg.serialize() {
        let mut clients_lock = clients_for_shutdown.lock().unwrap();
        let mut i = 0;
        while i < clients_lock.len() {
            if let Err(_) = clients_lock[i].write_all(&serialized) {
                clients_lock.remove(i);
            } else {
                i += 1;
            }
        }
    }
    
    // Gracefully stop the audio stream
    drop(_stream);
    println!("Audio capture stopped");
    println!("Server shutdown complete");
}

/// Get the audio configuration from the default input device
/// This is used to send config to clients when they connect
fn get_audio_config() -> Result<AudioConfig> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No default input device"))?;

    let supported_config = device
        .default_input_config()
        .map_err(|e| anyhow::anyhow!("Failed to get default input config: {e}"))?;

    println!("Recording config: {:?}", &supported_config);

    let sample_format = supported_config.sample_format();
    let config: StreamConfig = supported_config.into();

    Ok(AudioConfig {
        sample_rate: config.sample_rate.0,
        channels: config.channels,
        sample_format: SharedSampleFormat::from(sample_format),
    })
}

/// Start audio capture from the default input device
/// Returns the audio stream and sends audio data through the channel
fn start_default_input_capture(tx: mpsc::Sender<Message>) -> Result<cpal::Stream> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No default input device"))?;

    let supported_config = device
        .default_input_config()
        .map_err(|e| anyhow::anyhow!("Failed to get default input config: {e}"))?;

    let sample_format = supported_config.sample_format();
    let config: StreamConfig = supported_config.into();

    let err_fn = |err| eprintln!("an error occurred on stream: {err}");

    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _| on_input_data_f32(data, &tx),
            err_fn,
            None,
        )?,
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _| on_input_data_i16(data, &tx),
            err_fn,
            None,
        )?,
        SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _| on_input_data_u16(data, &tx),
            err_fn,
            None,
        )?,
        _ => anyhow::bail!("Unsupported sample format"),
    };

    stream.play()?;
    Ok(stream)
}

/// Convert f32 audio samples to bytes and send as AudioData message
fn on_input_data_f32(data: &[f32], tx: &mpsc::Sender<Message>) {
    let mut out = Vec::with_capacity(data.len() * 4);
    for &s in data {
        out.extend_from_slice(&s.to_le_bytes());
    }
    // println!("Captured {} samples", out.len());
    let _ = tx.send(Message::AudioData(out));
}

/// Convert i16 audio samples to bytes and send as AudioData message
fn on_input_data_i16(data: &[i16], tx: &mpsc::Sender<Message>) {
    let mut out = Vec::with_capacity(data.len() * 2);
    for &s in data {
        out.extend_from_slice(&s.to_le_bytes());
    }
    let _ = tx.send(Message::AudioData(out));
}

/// Convert u16 audio samples to bytes and send as AudioData message
fn on_input_data_u16(data: &[u16], tx: &mpsc::Sender<Message>) {
    let mut out = Vec::with_capacity(data.len() * 2);
    for &s in data {
        out.extend_from_slice(&s.to_le_bytes());
    }
    let _ = tx.send(Message::AudioData(out));
}

/// Accept incoming client connections and send them the audio config
/// Adds connected clients to the client list after sending config
fn accept_loop(clients: Arc<Mutex<Vec<TcpStream>>>, audio_config: AudioConfig, port: u16, running: Arc<AtomicBool>) {
    let listener = match TcpListener::bind(("0.0.0.0", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind TCP listener: {e}");
            return;
        }
    };
    println!("TCP server listening on 0.0.0.0:{}", port);
    
    // Set a timeout on the listener to allow checking the running flag
    listener.set_nonblocking(true).expect("Failed to set non-blocking");
    
    while running.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, addr)) => {
                let _ = stream.set_nodelay(true);
                println!("Client connected: {:?}", addr);
                
                // Send audio config to the newly connected client
                let config_msg = Message::Config(audio_config.clone());
                match config_msg.serialize() {
                    Ok(serialized) => {
                        if let Err(e) = stream.write_all(&serialized) {
                            eprintln!("Failed to send config to client: {e}");
                            continue;
                        }
                        println!("Sent audio config to client");
                    }
                    Err(e) => {
                        eprintln!("Failed to serialize config: {e}");
                        continue;
                    }
                }
                
                // Add client to the list after successfully sending config
                clients.lock().unwrap().push(stream);
                let client_count = clients.lock().unwrap().len();
                println!("Total clients connected: {}", client_count);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // No connection available, sleep briefly and continue
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(e) => {
                if running.load(Ordering::SeqCst) {
                    eprintln!("Accept error: {e}");
                }
                break;
            }
        }
    }
    
    println!("Accept loop stopped");
}

/// Broadcast messages to all connected clients
/// Serializes messages and sends them over TCP, removing disconnected clients
fn broadcast_loop(rx: mpsc::Receiver<Message>, clients: Arc<Mutex<Vec<TcpStream>>>, running: Arc<AtomicBool>) {
    while running.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(message) => {
                // Serialize the message to bytes
                let serialized = match message.serialize() {
                    Ok(data) => data,
                    Err(e) => {
                        eprintln!("Failed to serialize message: {e}");
                        continue;
                    }
                };

                let mut lock = match clients.lock() {
                    Ok(lock) => lock,
                    Err(_) => {
                        eprintln!("Mutex poisoned, skipping broadcast");
                        continue;
                    }
                };
                let mut i = 0;
                while i < lock.len() {
                    let client_addr = lock[i].peer_addr().map(|addr| addr.to_string()).unwrap_or_else(|_| "unknown".to_string());
                    let write_res = lock[i].write_all(&serialized);
                    if write_res.is_err() {
                        println!("Client disconnected: {:?}", client_addr);
                        let _ = lock[i].shutdown(std::net::Shutdown::Both);
                        lock.remove(i);
                        let remaining_clients = lock.len();
                        println!("Remaining clients: {}", remaining_clients);
                    } else {
                        i += 1;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Timeout is expected, continue checking running flag
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // Channel closed, exit gracefully
                break;
            }
        }
    }
    
    println!("Broadcast loop stopped");
}

/// Periodically check client connections to detect disconnections
/// Sends a small ping message to detect broken connections
fn health_check_loop(clients: Arc<Mutex<Vec<TcpStream>>>, running: Arc<AtomicBool>) {
    while running.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_secs(5)); // Check every 5 seconds
        
        if !running.load(Ordering::SeqCst) {
            break;
        }
        
        let mut lock = match clients.lock() {
            Ok(lock) => lock,
            Err(_) => {
                eprintln!("Mutex poisoned, skipping health check");
                continue;
            }
        };
        let mut i = 0;
        while i < lock.len() {
            let client_addr = lock[i].peer_addr().map(|addr| addr.to_string()).unwrap_or_else(|_| "unknown".to_string());
            
            // Try to send a small ping (empty audio data message)
            let ping_msg = Message::AudioData(vec![]);
            if let Ok(serialized) = ping_msg.serialize() {
                let write_res = lock[i].write_all(&serialized);
                if write_res.is_err() {
                    println!("Client disconnected (health check): {:?}", client_addr);
                    let _ = lock[i].shutdown(std::net::Shutdown::Both);
                    lock.remove(i);
                    let remaining_clients = lock.len();
                    println!("Remaining clients: {}", remaining_clients);
                } else {
                    i += 1;
                }
            } else {
                // If we can't serialize, remove the client
                println!("Client removed (serialization error): {:?}", client_addr);
                let _ = lock[i].shutdown(std::net::Shutdown::Both);
                lock.remove(i);
            }
        }
    }
    
    println!("Health check loop stopped");
}

/// Retrieve the current local IP address of the machine
fn get_local_ip() -> Option<std::net::IpAddr> {
    // Connect to a public IP address (doesn't actually send data)
    // This will use the default outbound interface
    let udp_socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    udp_socket.connect("8.8.8.8:80").ok()?;
    udp_socket.local_addr().ok().map(|addr| addr.ip())
}