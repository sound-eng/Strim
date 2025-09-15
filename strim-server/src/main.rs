use anyhow::Result;
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::time::Duration;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;

use strim_shared::{AudioConfig, AudioSample, Message, SampleFormat as SharedSampleFormat};

mod cli_commands;

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

    let shared_sample_format = SharedSampleFormat::try_from(sample_format)?;

    Ok(AudioConfig {
        sample_rate: config.sample_rate.0,
        channels: config.channels,
        sample_format: shared_sample_format,
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
            move |data: &[f32], _| on_input_data(data, &tx),
            err_fn,
            None,
        )?,
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _| on_input_data(data, &tx),
            err_fn,
            None,
        )?,
        SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _| on_input_data(data, &tx),
            err_fn,
            None,
        )?,
        SampleFormat::I32 => device.build_input_stream(
            &config, 
            move |data: &[i32], _| on_input_data(data, &tx), 
            err_fn, 
            None
        )?,
        _ => anyhow::bail!("Unsupported sample format"),
    };

    stream.play()?;
    Ok(stream)
}

/// Convert f32 audio samples to bytes and send as AudioData message
fn on_input_data<T: AudioSample>(data: &[T], tx: &mpsc::Sender<Message>) {
    let mut out = Vec::with_capacity(data.len() * T::BYTE_SIZE);
    for &s in data {
        out.extend_from_slice(&s.to_le_bytes().as_ref());
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

#[cfg(test)]
mod tests {
    use super::*;
    use strim_shared::{Message, AudioConfig, SampleFormat};
    use std::sync::{mpsc, Arc, Mutex, atomic::{AtomicBool, Ordering}};
    use std::net::{TcpListener, TcpStream};
    use std::io::{Read, Write};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_get_local_ip() {
        let ip = get_local_ip();
        // We can't guarantee what the IP will be, but it should be some valid IP
        // This test mainly checks that the function doesn't panic
        match ip {
            Some(addr) => {
                // Should be a valid IP address
                assert!(addr.is_ipv4() || addr.is_ipv6());
                // Should not be the loopback or unspecified address in normal cases
                println!("Local IP detected: {}", addr);
            }
            None => {
                // In some environments (like CI), this might fail
                println!("Could not detect local IP (this is sometimes expected in test environments)");
            }
        }
    }

    #[test]
    fn test_on_input_data_f32() {
        let (tx, rx) = mpsc::channel::<Message>();
        let test_data: Vec<f32> = vec![0.0, 0.5, -0.5, 1.0, -1.0];
        
        // Call the function
        on_input_data(&test_data, &tx);
        
        // Receive the message
        let message = rx.recv().expect("Should receive a message");
        
        match message {
            Message::AudioData(bytes) => {
                // Should have correct number of bytes (5 f32s * 4 bytes each = 20 bytes)
                assert_eq!(bytes.len(), 20);
                
                // Convert back to f32s to verify
                let mut restored_data = Vec::new();
                for chunk in bytes.chunks(4) {
                    let array: [u8; 4] = [chunk[0], chunk[1], chunk[2], chunk[3]];
                    restored_data.push(f32::from_le_bytes(array));
                }
                
                assert_eq!(restored_data, test_data);
            }
            _ => panic!("Should receive AudioData message"),
        }
    }

    #[test]
    fn test_on_input_data_i16() {
        let (tx, rx) = mpsc::channel::<Message>();
        let test_data: Vec<i16> = vec![0, 1000, -1000, i16::MAX, i16::MIN];
        
        // Call the function
        on_input_data(&test_data, &tx);
        
        // Receive the message
        let message = rx.recv().expect("Should receive a message");
        
        match message {
            Message::AudioData(bytes) => {
                // Should have correct number of bytes (5 i16s * 2 bytes each = 10 bytes)
                assert_eq!(bytes.len(), 10);
                
                // Convert back to i16s to verify
                let mut restored_data = Vec::new();
                for chunk in bytes.chunks(2) {
                    let array: [u8; 2] = [chunk[0], chunk[1]];
                    restored_data.push(i16::from_le_bytes(array));
                }
                
                assert_eq!(restored_data, test_data);
            }
            _ => panic!("Should receive AudioData message"),
        }
    }

    /// Test helper to create a mock TCP connection pair
    fn create_mock_tcp_pair() -> (TcpListener, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Should bind to localhost");
        let port = listener.local_addr().expect("Should get local address").port();
        (listener, port)
    }

    #[test]
    fn test_client_connection_and_config_send() {
        let (listener, port) = create_mock_tcp_pair();
        
        // Create test audio config
        let test_config = AudioConfig {
            sample_rate: 44100,
            channels: 2,
            sample_format: SampleFormat::F32,
        };
        
        // Spawn a thread to accept connection and send config
        let config_clone = test_config.clone();
        let server_handle = thread::spawn(move || {
            let (mut stream, _addr) = listener.accept().expect("Should accept connection");
            
            // Send config message
            let config_msg = Message::Config(config_clone);
            let serialized = config_msg.serialize().expect("Should serialize config");
            stream.write_all(&serialized).expect("Should send config");
            
            // Send a test audio data message
            let audio_msg = Message::AudioData(vec![1, 2, 3, 4]);
            let serialized = audio_msg.serialize().expect("Should serialize audio data");
            stream.write_all(&serialized).expect("Should send audio data");
        });
        
        // Connect as client
        let mut client_stream = TcpStream::connect(format!("127.0.0.1:{}", port))
            .expect("Should connect to server");
        
        // Read and deserialize config message
        let mut buffer = vec![0u8; 1024];
        let bytes_read = client_stream.read(&mut buffer).expect("Should read data");
        buffer.truncate(bytes_read);
        
        let (config_message, remaining) = Message::deserialize(&buffer)
            .expect("Should deserialize config message");
        
        match config_message {
            Message::Config(received_config) => {
                assert_eq!(received_config.sample_rate, test_config.sample_rate);
                assert_eq!(received_config.channels, test_config.channels);
                assert_eq!(received_config.sample_format, test_config.sample_format);
            }
            _ => panic!("Should receive config message first"),
        }
        
        // Read audio data message from remaining bytes or read more
        let audio_data = if remaining.is_empty() {
            let mut buffer = vec![0u8; 1024];
            let bytes_read = client_stream.read(&mut buffer).expect("Should read more data");
            buffer.truncate(bytes_read);
            buffer
        } else {
            remaining.to_vec()
        };
        
        let (audio_message, _) = Message::deserialize(&audio_data)
            .expect("Should deserialize audio message");
        
        match audio_message {
            Message::AudioData(data) => {
                assert_eq!(data, vec![1, 2, 3, 4]);
            }
            _ => panic!("Should receive audio data message"),
        }
        
        server_handle.join().expect("Server thread should complete");
    }

    #[test]
    fn test_broadcast_loop_basic() {
        let (tx, rx) = mpsc::channel::<Message>();
        let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
        let running = Arc::new(AtomicBool::new(true));
        
        // Create a mock client connection
        let (listener, port) = create_mock_tcp_pair();
        
        // Start the broadcast loop in a separate thread
        let clients_clone = Arc::clone(&clients);
        let running_clone = Arc::clone(&running);
        let broadcast_handle = thread::spawn(move || {
            broadcast_loop(rx, clients_clone, running_clone);
        });
        
        // Connect a mock client
        let client_handle = thread::spawn(move || {
            let (stream, _addr) = listener.accept().expect("Should accept client");
            
            // Add client to the list
            clients.lock().unwrap().push(stream);
            
            // Wait a bit for the broadcast to happen
            thread::sleep(Duration::from_millis(100));
        });
        
        // Connect to the server as a client
        let mut client_stream = TcpStream::connect(format!("127.0.0.1:{}", port))
            .expect("Should connect as client");
        
        // Wait for client to be added
        client_handle.join().expect("Client thread should complete");
        
        // Send a test message
        let test_msg = Message::AudioData(vec![10, 20, 30, 40]);
        tx.send(test_msg).expect("Should send test message");
        
        // Give broadcast loop time to process
        thread::sleep(Duration::from_millis(50));
        
        // Try to read the message as the client
        client_stream.set_nonblocking(true).expect("Should set non-blocking");
        let mut buffer = vec![0u8; 1024];
        
        // We might get the message, but this test mainly verifies the broadcast doesn't panic
        let _ = client_stream.read(&mut buffer);
        
        // Stop the broadcast loop
        running.store(false, Ordering::SeqCst);
        drop(tx); // Close the channel to make broadcast_loop exit
        
        broadcast_handle.join().expect("Broadcast thread should complete");
    }

    #[test]
    fn test_message_channel_communication() {
        let (tx, rx) = mpsc::channel::<Message>();
        
        // Test sending various message types
        let messages = vec![
            Message::AudioData(vec![1, 2, 3]),
            Message::Config(AudioConfig {
                sample_rate: 48000,
                channels: 1,
                sample_format: SampleFormat::I16,
            }),
            Message::Error("Test error".to_string()),
        ];
        
        // Send messages
        for msg in messages.clone() {
            tx.send(msg).expect("Should send message");
        }
        
        // Receive and verify messages
        for (i, expected_msg) in messages.iter().enumerate() {
            let received_msg = rx.recv().expect("Should receive message");
            
            match (expected_msg, &received_msg) {
                (Message::AudioData(expected), Message::AudioData(received)) => {
                    assert_eq!(expected, received, "AudioData mismatch at index {}", i);
                }
                (Message::Config(expected), Message::Config(received)) => {
                    assert_eq!(expected.sample_rate, received.sample_rate);
                    assert_eq!(expected.channels, received.channels);
                    assert_eq!(expected.sample_format, received.sample_format);
                }
                (Message::Error(expected), Message::Error(received)) => {
                    assert_eq!(expected, received, "Error message mismatch at index {}", i);
                }
                _ => panic!("Message type mismatch at index {}", i),
            }
        }
    }
}
