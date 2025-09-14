use anyhow::Result;
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::io::Read;
use std::net::TcpStream;
use std::sync::{mpsc, Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::Duration;

use strim_shared::{Message, AudioConfig, SampleFormat as SharedSampleFormat, AudioSample};

// Message types for communication between threads
#[derive(Debug)]
enum ConnectionEvent {
    Connected(TcpStream),
    Disconnected,
    Shutdown,
}

mod cli_commands;

/// Attempts to connect to the server with retry logic
/// Returns Ok(stream) on successful connection, Err on permanent failure
fn connect_with_retry(host: &str, port: u16, running: Arc<AtomicBool>) -> Result<TcpStream> {
    let mut attempt = 1;
    
    loop {
        if !running.load(Ordering::SeqCst) {
            return Err(anyhow::anyhow!("Shutdown requested"));
        }
        
        println!("Connection attempt {} to {}:{}", attempt, host, port);
        
        match TcpStream::connect((host, port)) {
            Ok(stream) => {
                stream.set_nodelay(true)?;
                println!("Successfully connected to {}:{}", host, port);
                return Ok(stream);
            }
            Err(e) => {
                if !running.load(Ordering::SeqCst) {
                    return Err(anyhow::anyhow!("Shutdown requested"));
                }
                
                println!("Connection attempt {} failed: {}. Retrying in 5 seconds...", attempt, e);
                attempt += 1;
                
                // Sleep for 5 seconds, but check running status every 100ms
                for _ in 0..50 {
                    if !running.load(Ordering::SeqCst) {
                        return Err(anyhow::anyhow!("Shutdown requested"));
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
}

fn main() -> Result<()> {
    let args = cli_commands::Cli::parse();
    
    // Set up graceful shutdown handling
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);
    
    ctrlc::set_handler(move || {
        println!("\nReceived Ctrl+C, shutting down gracefully...");
        running_clone.store(false, Ordering::SeqCst);
    })?;
    
    println!("Starting audio streaming client");
    println!("Connecting to server at {}:{}", args.host, args.port);
    println!("Press Ctrl+C to disconnect gracefully");
    
    // Channel for communication between connection and audio threads
    let (connection_tx, connection_rx) = mpsc::channel::<ConnectionEvent>();
    let (audio_tx, audio_rx) = mpsc::channel::<Message>();
    
    // Spawn connection management thread
    let host = args.host.clone();
    let port = args.port;
    let running_conn = Arc::clone(&running);
    let audio_tx_clone = audio_tx.clone();
    let connection_tx_clone = connection_tx.clone();
    thread::spawn(move || {
        connection_manager(host, port, running_conn, connection_tx_clone, audio_tx_clone);
    });
    
    // Main loop: handle connection events and manage audio playback
    let mut current_stream: Option<TcpStream> = None;
    let mut audio_handle: Option<thread::JoinHandle<()>> = None;
    let mut connection_manager_handle: Option<thread::JoinHandle<()>> = None;
    let mut audio_tx = audio_tx;
    let mut audio_rx = audio_rx;
    
    while running.load(Ordering::SeqCst) {
        match connection_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(ConnectionEvent::Connected(stream)) => {
                println!("Connection established, starting audio playback");
                current_stream = Some(stream);
                
                // Start audio playback thread with current receiver
                let running_audio = Arc::clone(&running);
                let handle = thread::spawn(move || {
                    if let Err(e) = start_audio_playback(audio_rx, running_audio) {
                        eprintln!("Audio playback error: {}", e);
                    }
                });
                audio_handle = Some(handle);
                
                // Create new audio channel for potential next connection
                let (new_audio_tx, new_audio_rx) = mpsc::channel::<Message>();
                audio_tx = new_audio_tx;
                audio_rx = new_audio_rx;
            }
            Ok(ConnectionEvent::Disconnected) => {
                println!("Main thread received ConnectionEvent::Disconnected");
                println!("Connection lost, will attempt to reconnect...");
                current_stream = None;
                
                // Stop audio playback
                if let Some(handle) = audio_handle.take() {
                    drop(handle);
                }
                
                // Restart connection manager to attempt reconnection
                let host = args.host.clone();
                let port = args.port;
                let running_conn = Arc::clone(&running);
                let audio_tx_clone = audio_tx.clone();
                let connection_tx_clone = connection_tx.clone();
                let handle = thread::spawn(move || {
                    connection_manager(host, port, running_conn, connection_tx_clone, audio_tx_clone);
                });
                connection_manager_handle = Some(handle);
            }
            Ok(ConnectionEvent::Shutdown) => {
                println!("Shutdown requested");
                break;
            }
            Err(_) => {
                println!("Connection channel closed");
                break;
            }
        }
    }
    
    // Cleanup
    if let Some(handle) = audio_handle {
        let _ = handle.join();
    }
    
    if let Some(handle) = connection_manager_handle {
        println!("Dropping connection manager thread handle...");
        drop(handle);
        println!("Connection manager thread handle dropped");
    }
    
    println!("Client disconnected");
    Ok(())
}

/// Manages connection lifecycle with automatic reconnection
fn connection_manager(
    host: String,
    port: u16,
    running: Arc<AtomicBool>,
    connection_tx: mpsc::Sender<ConnectionEvent>,
    audio_tx: mpsc::Sender<Message>,
) {
    println!("Connection manager started for {}:{}", host, port);
    while running.load(Ordering::SeqCst) {
        // Try to connect
        match connect_with_retry(&host, port, Arc::clone(&running)) {
            Ok(stream) => {
                // Connection successful, notify main thread
                if connection_tx.send(ConnectionEvent::Connected(stream.try_clone().unwrap())).is_err() {
                    break; // Main thread is shutting down
                }
                
                // Start network read loop
                let running_network = Arc::clone(&running);
                let audio_tx_clone = audio_tx.clone();
                let connection_tx_clone = connection_tx.clone();
                
                thread::spawn(move || {
                    network_read_loop(stream, audio_tx_clone, running_network, connection_tx_clone);
                });
                
                // Exit this connection manager - the main thread will spawn a new one
                // when it receives ConnectionEvent::Disconnected
                println!("Connection manager exiting after successful connection");
                break;
            }
            Err(e) => {
                if running.load(Ordering::SeqCst) {
                    eprintln!("Failed to connect: {}", e);
                    // Continue the loop to try reconnecting again
                    continue;
                } else {
                    // Shutdown was requested, exit gracefully
                    println!("Connection manager: shutdown requested, exiting");
                    break;
                }
            }
        }
    }
    
    // Connection manager thread exits - no need to send shutdown event
    // The main thread will handle shutdown via Ctrl+C
}

/// Read messages from the network and deserialize them
/// Sends deserialized messages to the audio playback thread
fn network_read_loop(
    mut stream: TcpStream, 
    tx: mpsc::Sender<Message>, 
    running: Arc<AtomicBool>,
    connection_tx: mpsc::Sender<ConnectionEvent>,
) {
    // size 4096 and 2*4096 wasn't enough when connecting through wifi. consider increasing even more:
    let mut buffer = [0u8; 4*4096]; 
    let mut message_buffer = Vec::new();
    
    while running.load(Ordering::SeqCst) {
        match stream.read(&mut buffer) {
            Ok(0) => {
                println!("Server disconnected");
                // Notify connection manager about disconnection
                println!("Sending ConnectionEvent::Disconnected");
                let _ = connection_tx.send(ConnectionEvent::Disconnected);
                break;
            }
            Ok(n) => {
                // println!("Got data: {n}");
                // Add new data to our message buffer
                message_buffer.extend_from_slice(&buffer[..n]);
                
                // Try to deserialize complete messages from the buffer
                loop {
                    match Message::deserialize(&message_buffer) {
                        Ok((message, remaining)) => {
                            // Successfully deserialized a message
                            if tx.send(message).is_err() {
                                return; // Channel closed, exit
                            }
                            // Update buffer to remaining data
                            message_buffer = remaining.to_vec();
                            // println!(" >> Got Message, remaining: {}", &message_buffer.len());
                        }
                        Err(_) => {
                            // println!(" >> Partial data: {}", &message_buffer.len());
                            // Not enough data for a complete message, wait for more
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                if running.load(Ordering::SeqCst) {
                    eprintln!("Network read error: {e}");
                    // Notify connection manager about disconnection
                    println!("Sending ConnectionEvent::Disconnected due to error");
                    let _ = connection_tx.send(ConnectionEvent::Disconnected);
                }
                break;
            }
        }
    }
    
    // Gracefully close the connection
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

/// Start audio playback, handling both config and audio data messages
fn start_audio_playback(rx: mpsc::Receiver<Message>, running: Arc<AtomicBool>) -> Result<()> {
    // Wait for config message from server first
    let audio_config = match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Message::Config(config)) => {
            println!("Received audio config from server: {:?}", config);
            config
        }
        Ok(Message::Error(err)) => {
            eprintln!("Server error: {}", err);
            return Err(anyhow::anyhow!("Server error: {}", err));
        }
        Ok(Message::AudioData(_)) => {
            eprintln!("Received audio data before config, using default config");
            // Use default config if we get audio data first
            AudioConfig {
                sample_rate: 44100,
                channels: 2,
                sample_format: SharedSampleFormat::F32,
            }
        }
        Err(e) => {
            eprintln!("Failed to receive config: {e}");
            return Err(anyhow::anyhow!("Failed to receive config: {}", e));
        }
    };

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("No default output device"))?;

    // Create stream config from received audio config
    let config = StreamConfig {
        channels: audio_config.channels,
        sample_rate: cpal::SampleRate(audio_config.sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let sample_format: SampleFormat = audio_config.sample_format.into();
    
    println!("Using server audio config: {:?}", audio_config);

    let err_fn = |err| eprintln!("Audio playback error: {err}");

    // Create a buffer to hold incoming audio data
    let audio_buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let buffer_for_network = Arc::clone(&audio_buffer);
    let buffer_for_audio = Arc::clone(&audio_buffer);

    // Spawn thread to receive network messages (config already received above)
    let running_audio = Arc::clone(&running);
    thread::spawn(move || {
        println!("Audio thread started");
        while running_audio.load(Ordering::SeqCst) {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(message) => {
                    match message {
                        Message::AudioData(data) => {
                            let mut buffer = buffer_for_network.lock().unwrap();
                            buffer.extend_from_slice(&data);
                        }
                        Message::Config(config) => {
                            println!("Received updated config: {:?}", config);
                            // Could handle config updates here if needed
                            // For now, we'll just log it since we already configured the stream
                        }
                        Message::Error(err) => {
                            eprintln!("Server error: {}", err);
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Timeout - check if we should continue running
                    continue;
                }
                Err(_) => {
                    // Channel closed, exit gracefully
                    println!("Audio thread: channel closed, exiting");
                    break;
                }
            }
        }
        println!("Audio thread exiting");
    });

    let shared_sample_format = SharedSampleFormat::try_from(sample_format)?;

    let stream = match shared_sample_format {
        SharedSampleFormat::F32 => device.build_output_stream(
            &config,
            move |data: &mut [f32], _| on_output_data(data, &buffer_for_audio),
            err_fn,
            None,
        )?,
        SharedSampleFormat::I16 => device.build_output_stream(
            &config,
            move |data: &mut [i16], _| on_output_data(data, &buffer_for_audio),
            err_fn,
            None,
        )?,
        SharedSampleFormat::U16 => device.build_output_stream(
            &config,
            move |data: &mut [u16], _| on_output_data(data, &buffer_for_audio),
            err_fn,
            None,
        )?,
        SharedSampleFormat::I32 => device.build_output_stream(
            &config,
            move |data: &mut [i32], _| on_output_data(data, &buffer_for_audio),
            err_fn, 
            None
        )?,
        _ => anyhow::bail!("Unsupported sample format"),
    };

    stream.play()?;
    
    // Keep the main thread alive until shutdown signal
    while running.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(100));
    }
    
    // Gracefully stop the audio stream
    drop(stream);
    println!("Audio playback stopped");
    
    Ok(())
}


// Mark: Output callback

fn on_output_data<T: AudioSample>(data: &mut [T], buffer: &Arc<Mutex<Vec<u8>>>) {
    let mut audio_buffer = buffer.lock().unwrap();
    let bytes_needed = data.len() * T::BYTE_SIZE;
    
    if audio_buffer.len() < bytes_needed {
        // Not enough data, fill with silence
        data.fill(T::default());
        return;
    }
    
    // Convert bytes to u16 samples
    for (i, sample_bytes) in audio_buffer[..bytes_needed].chunks(T::BYTE_SIZE).enumerate() {
        debug_assert!(i < data.len(), "Chunk index {} exceeds data length {}", i, data.len());
        debug_assert_eq!(sample_bytes.len(), T::BYTE_SIZE, "Chunk has {} bytes instead of 4", sample_bytes.len());

        data[i] = T::from_le_bytes(sample_bytes);
    }

    // Remove processed bytes
    audio_buffer.drain(..bytes_needed);
}
