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
    let initial_connection_handle = thread::spawn(move || {
        connection_manager(host, port, running_conn, connection_tx_clone, audio_tx_clone);
    });
    
    // Main loop: handle connection events and manage audio playback
    let mut audio_handle: Option<thread::JoinHandle<()>> = None;
    let mut connection_manager_handle: Option<thread::JoinHandle<()>> = Some(initial_connection_handle);
    let mut audio_tx = audio_tx;
    let mut audio_rx = audio_rx;
    
    while running.load(Ordering::SeqCst) {
        match connection_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(ConnectionEvent::Connected(_stream)) => {
                println!("Connection established, starting audio playback");
                
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
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Timeout occurred, continue loop to check running flag
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                println!("Connection channel closed");
                break;
            }
        }
    }
    
    // Cleanup
    println!("Starting cleanup...");
    
    if let Some(handle) = audio_handle {
        println!("Joining audio thread...");
        let _ = handle.join();
    }
    
    if let Some(handle) = connection_manager_handle {
        println!("Joining connection manager thread...");
        // Give the connection manager thread a moment to see the running flag change
        thread::sleep(Duration::from_millis(200));
        let _ = handle.join();
        println!("Connection manager thread joined");
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
    // Make the stream non-blocking to allow periodic shutdown checks
    stream.set_nonblocking(true).expect("Failed to set non-blocking");
    
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
                if e.kind() == std::io::ErrorKind::WouldBlock {
                    // No data available right now, sleep briefly and continue
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                
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
    println!("Network read loop exiting, closing connection");
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

#[cfg(test)]
mod tests {
    use super::*;
    use strim_shared::{Message, AudioConfig, SampleFormat as SharedSampleFormat};
    use std::sync::{mpsc, Arc, Mutex, atomic::{AtomicBool, Ordering}};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_connection_event_enum() {
        // Test that ConnectionEvent can be created and sent through channels
        let (tx, rx) = mpsc::channel::<ConnectionEvent>();
        
        // Test Connected event
        let listener = TcpListener::bind("127.0.0.1:0").expect("Should bind");
        let port = listener.local_addr().unwrap().port();
        
        let tx_clone = tx.clone();
        thread::spawn(move || {
            let (stream, _) = listener.accept().expect("Should accept");
            tx_clone.send(ConnectionEvent::Connected(stream)).expect("Should send Connected event");
        });
        
        let _client_stream = TcpStream::connect(format!("127.0.0.1:{}", port))
            .expect("Should connect");
        
        let event = rx.recv_timeout(Duration::from_millis(100)).expect("Should receive event");
        match event {
            ConnectionEvent::Connected(_) => (), // Success
            _ => panic!("Should receive Connected event"),
        }
        
        // Test Disconnected event
        tx.send(ConnectionEvent::Disconnected).expect("Should send Disconnected event");
        let event = rx.recv_timeout(Duration::from_millis(100)).expect("Should receive event");
        match event {
            ConnectionEvent::Disconnected => (), // Success
            _ => panic!("Should receive Disconnected event"),
        }
    }

    #[test]
    fn test_connect_with_retry_success() {
        let running = Arc::new(AtomicBool::new(true));
        
        // Create a server that accepts connections
        let listener = TcpListener::bind("127.0.0.1:0").expect("Should bind");
        let port = listener.local_addr().unwrap().port();
        
        thread::spawn(move || {
            let (_stream, _addr) = listener.accept().expect("Should accept connection");
        });
        
        // Test successful connection
        let result = connect_with_retry("127.0.0.1", port, running);
        assert!(result.is_ok(), "Connection should succeed");
    }

    #[test]
    fn test_connect_with_retry_shutdown() {
        let running = Arc::new(AtomicBool::new(true));
        
        let running_clone = Arc::clone(&running);
        thread::spawn(move || {
            // Shutdown after a short delay
            thread::sleep(Duration::from_millis(50));
            running_clone.store(false, Ordering::SeqCst);
        });
        
        // Try to connect to a non-existent server
        let result = connect_with_retry("127.0.0.1", 12345, running); // Use a likely unused port
        assert!(result.is_err(), "Should fail due to shutdown");
    }

    #[test]
    fn test_on_output_data_f32() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let mut output_data: [f32; 4] = [0.0; 4];
        
        // Test with empty buffer (should fill with silence)
        on_output_data(&mut output_data, &buffer);
        assert_eq!(output_data, [0.0, 0.0, 0.0, 0.0]);
        
        // Add some audio data to buffer (4 f32s = 16 bytes)
        let test_samples = [0.5f32, -0.5f32, 1.0f32, -1.0f32];
        let mut test_bytes = Vec::new();
        for sample in test_samples {
            test_bytes.extend_from_slice(&sample.to_le_bytes());
        }
        
        {
            let mut buf = buffer.lock().unwrap();
            buf.extend_from_slice(&test_bytes);
        }
        
        // Test with sufficient data
        on_output_data(&mut output_data, &buffer);
        assert_eq!(output_data, test_samples);
        
        // Buffer should be empty now
        assert!(buffer.lock().unwrap().is_empty());
    }

    #[test]
    fn test_on_output_data_i16() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let mut output_data: [i16; 3] = [0; 3];
        
        // Test with insufficient data
        {
            let mut buf = buffer.lock().unwrap();
            buf.extend_from_slice(&[1u8, 2u8]); // Only 2 bytes, need 6
        }
        
        on_output_data(&mut output_data, &buffer);
        assert_eq!(output_data, [0, 0, 0]); // Should be filled with silence
        
        // Add sufficient data
        let test_samples = [100i16, -200i16, 300i16];
        let mut test_bytes = Vec::new();
        for sample in test_samples {
            test_bytes.extend_from_slice(&sample.to_le_bytes());
        }
        
        {
            let mut buf = buffer.lock().unwrap();
            buf.clear();
            buf.extend_from_slice(&test_bytes);
        }
        
        on_output_data(&mut output_data, &buffer);
        assert_eq!(output_data, test_samples);
    }

    #[test]
    fn test_audio_config_handling() {
        let (tx, rx) = mpsc::channel::<Message>();
        
        // Test different config scenarios
        let configs = vec![
            AudioConfig {
                sample_rate: 44100,
                channels: 2,
                sample_format: SharedSampleFormat::F32,
            },
            AudioConfig {
                sample_rate: 48000,
                channels: 1,
                sample_format: SharedSampleFormat::I16,
            },
        ];
        
        for config in configs {
            let config_msg = Message::Config(config.clone());
            tx.send(config_msg).expect("Should send config");
            
            let received = rx.recv().expect("Should receive config");
            match received {
                Message::Config(received_config) => {
                    assert_eq!(received_config.sample_rate, config.sample_rate);
                    assert_eq!(received_config.channels, config.channels);
                    assert_eq!(received_config.sample_format, config.sample_format);
                }
                _ => panic!("Should receive Config message"),
            }
        }
    }

    #[test]
    fn test_audio_buffer_management() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        
        // Test adding data to buffer
        let test_data = vec![1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, 8u8];
        {
            let mut buf = buffer.lock().unwrap();
            buf.extend_from_slice(&test_data);
        }
        
        assert_eq!(buffer.lock().unwrap().len(), 8);
        
        // Test partial consumption (simulate on_output_data)
        let mut output: [i16; 2] = [0; 2]; // Will consume 4 bytes
        on_output_data(&mut output, &buffer);
        
        // Should have 4 bytes remaining
        assert_eq!(buffer.lock().unwrap().len(), 4);
        assert_eq!(*buffer.lock().unwrap(), vec![5u8, 6u8, 7u8, 8u8]);
    }

    #[test]
    fn test_network_message_handling() {
        let (tx, rx) = mpsc::channel::<Message>();
        let running = Arc::new(AtomicBool::new(true));
        let buffer = Arc::new(Mutex::new(Vec::new()));
        
        // Test receiving different message types
        let messages = vec![
            Message::Config(AudioConfig {
                sample_rate: 44100,
                channels: 2,
                sample_format: SharedSampleFormat::F32,
            }),
            Message::AudioData(vec![1, 2, 3, 4, 5, 6, 7, 8]),
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
                (Message::Config(expected), Message::Config(received)) => {
                    assert_eq!(expected.sample_rate, received.sample_rate);
                    assert_eq!(expected.channels, received.channels);
                    assert_eq!(expected.sample_format, received.sample_format);
                }
                (Message::AudioData(expected), Message::AudioData(received)) => {
                    assert_eq!(expected, received, "AudioData mismatch at index {}", i);
                    
                    // Simulate adding to buffer
                    {
                        let mut buf = buffer.lock().unwrap();
                        buf.extend_from_slice(received);
                    }
                }
                (Message::Error(expected), Message::Error(received)) => {
                    assert_eq!(expected, received, "Error message mismatch at index {}", i);
                }
                _ => panic!("Message type mismatch at index {}", i),
            }
        }
        
        // Check that audio data was added to buffer
        assert_eq!(buffer.lock().unwrap().len(), 8);
    }

    #[test]
    fn test_sample_format_conversion() {
        // Test all supported sample format conversions
        let formats = vec![
            SharedSampleFormat::F32,
            SharedSampleFormat::I16,
            SharedSampleFormat::U16,
            SharedSampleFormat::I32,
        ];
        
        for format in formats {
            let cpal_format: cpal::SampleFormat = format.into();
            let back_to_shared = SharedSampleFormat::try_from(cpal_format).expect("Should convert back");
            assert_eq!(format, back_to_shared, "Conversion failed for {:?}", format);
        }
    }

    #[test]
    fn test_connection_lifecycle() {
        let (connection_tx, connection_rx) = mpsc::channel::<ConnectionEvent>();
        
        // Simulate connection lifecycle events
        let events = vec![
            ConnectionEvent::Disconnected,
            // ConnectionEvent::Shutdown, // This variant is never constructed according to warnings
        ];
        
        for event in events {
            connection_tx.send(event).expect("Should send event");
            let received = connection_rx.recv().expect("Should receive event");
            
            match received {
                ConnectionEvent::Connected(_) => println!("Received Connected event"),
                ConnectionEvent::Disconnected => println!("Received Disconnected event"),
                ConnectionEvent::Shutdown => println!("Received Shutdown event"),
            }
        }
    }
}
