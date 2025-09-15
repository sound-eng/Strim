//! Integration tests for Strim client-server communication
//! 
//! These tests verify that the entire network protocol works correctly
//! between client and server components.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::Duration;

use strim_shared::{Message, AudioConfig, SampleFormat, DEFAULT_PORT};

/// Test that a basic client-server handshake works correctly
#[test]
fn test_client_server_handshake() {
    // Create a mock server
    let listener = TcpListener::bind("127.0.0.1:0").expect("Should bind to localhost");
    let server_port = listener.local_addr().expect("Should get server address").port();
    
    let test_config = AudioConfig {
        sample_rate: 44100,
        channels: 2,
        sample_format: SampleFormat::F32,
    };
    
    let config_clone = test_config.clone();
    let server_handle = thread::spawn(move || {
        println!("Integration test server starting...");
        
        // Accept connection
        let (mut stream, addr) = listener.accept().expect("Should accept connection");
        println!("Server accepted connection from: {:?}", addr);
        
        // Send config message
        let config_msg = Message::Config(config_clone);
        let serialized = config_msg.serialize().expect("Should serialize config");
        stream.write_all(&serialized).expect("Should send config");
        println!("Server sent config message");
        
        // Send some test audio data
        let audio_data = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let audio_msg = Message::AudioData(audio_data);
        let serialized = audio_msg.serialize().expect("Should serialize audio");
        stream.write_all(&serialized).expect("Should send audio data");
        println!("Server sent audio data");
        
        // Keep connection alive for a bit
        thread::sleep(Duration::from_millis(100));
    });
    
    // Create client
    println!("Integration test client connecting to port {}...", server_port);
    let mut client_stream = TcpStream::connect(format!("127.0.0.1:{}", server_port))
        .expect("Should connect to server");
    
    // Read config message
    let mut buffer = vec![0u8; 1024];
    let bytes_read = client_stream.read(&mut buffer).expect("Should read config data");
    buffer.truncate(bytes_read);
    
    let (config_message, remaining) = Message::deserialize(&buffer)
        .expect("Should deserialize config message");
    
    match config_message {
        Message::Config(received_config) => {
            assert_eq!(received_config.sample_rate, test_config.sample_rate);
            assert_eq!(received_config.channels, test_config.channels);
            assert_eq!(received_config.sample_format, test_config.sample_format);
            println!("Client received and verified config: {:?}", received_config);
        }
        _ => panic!("Should receive config message first"),
    }
    
    // Read audio data message
    let audio_data = if remaining.is_empty() {
        let mut buffer = vec![0u8; 1024];
        let bytes_read = client_stream.read(&mut buffer).expect("Should read audio data");
        buffer.truncate(bytes_read);
        buffer
    } else {
        remaining.to_vec()
    };
    
    let (audio_message, _) = Message::deserialize(&audio_data)
        .expect("Should deserialize audio message");
    
    match audio_message {
        Message::AudioData(data) => {
            assert_eq!(data, vec![1u8, 2, 3, 4, 5, 6, 7, 8]);
            println!("Client received and verified audio data: {:?}", data);
        }
        _ => panic!("Should receive audio data message"),
    }
    
    server_handle.join().expect("Server thread should complete");
    println!("Integration test client-server handshake completed successfully");
}

/// Test error message handling between client and server
#[test]
fn test_error_message_propagation() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Should bind");
    let port = listener.local_addr().unwrap().port();
    
    let error_message = "Test error from server";
    let error_clone = error_message.to_string();
    
    let server_handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("Should accept");
        
        // Send error message
        let error_msg = Message::Error(error_clone);
        let serialized = error_msg.serialize().expect("Should serialize error");
        stream.write_all(&serialized).expect("Should send error");
    });
    
    // Client connects and reads error
    let mut client_stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .expect("Should connect");
    
    let mut buffer = vec![0u8; 1024];
    let bytes_read = client_stream.read(&mut buffer).expect("Should read error");
    buffer.truncate(bytes_read);
    
    let (message, _) = Message::deserialize(&buffer)
        .expect("Should deserialize error message");
    
    match message {
        Message::Error(received_error) => {
            assert_eq!(received_error, error_message);
            println!("Successfully received error message: {}", received_error);
        }
        _ => panic!("Should receive error message"),
    }
    
    server_handle.join().expect("Server should complete");
}

/// Test multiple clients connecting to the same server
#[test] 
fn test_multiple_client_connections() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Should bind");
    let port = listener.local_addr().unwrap().port();
    
    let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
    let clients_clone = Arc::clone(&clients);
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);
    
    // Mock server that accepts multiple connections
    let server_handle = thread::spawn(move || {
        listener.set_nonblocking(true).expect("Should set non-blocking");
        
        while running_clone.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, addr)) => {
                    println!("Server accepted connection from: {:?}", addr);
                    clients_clone.lock().unwrap().push(stream);
                }
                Err(_) => {
                    // Non-blocking, so this is expected
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
    });
    
    // Connect multiple clients
    let num_clients = 3usize;
    let mut client_handles = Vec::new();
    
    for i in 0..num_clients {
        let client_handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(i as u64 * 10)); // Stagger connections
            TcpStream::connect(format!("127.0.0.1:{}", port))
                .expect("Should connect to server")
        });
        client_handles.push(client_handle);
    }
    
    // Collect all client streams
    let client_streams: Vec<_> = client_handles.into_iter()
        .map(|h| h.join().expect("Client thread should complete"))
        .collect();
    
    // Wait a bit for server to accept all connections
    thread::sleep(Duration::from_millis(100));
    
    // Check that server has accepted all connections
    let server_clients = clients.lock().unwrap();
    assert_eq!(server_clients.len(), num_clients, "Server should have accepted {} clients", num_clients);
    
    println!("Successfully connected {} clients to server", server_clients.len());
    
    // Shutdown server
    running.store(false, Ordering::SeqCst);
    server_handle.join().expect("Server should complete");
    
    // Clean up client streams
    drop(client_streams);
}

/// Test that large messages are handled correctly
#[test]
fn test_large_message_handling() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Should bind");
    let port = listener.local_addr().unwrap().port();
    
    // Create a large audio data message (1MB)
    let large_data = vec![42u8; 1024 * 1024];
    let large_data_clone = large_data.clone();
    
    let server_handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("Should accept");
        
        let large_msg = Message::AudioData(large_data_clone);
        let serialized = large_msg.serialize().expect("Should serialize large message");
        stream.write_all(&serialized).expect("Should send large message");
        
        println!("Server sent large message of {} bytes", serialized.len());
    });
    
    let mut client_stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .expect("Should connect");
    
    // Read the large message
    let mut buffer = Vec::new();
    
    // Read length prefix first
    let mut length_bytes = [0u8; 4];
    client_stream.read_exact(&mut length_bytes).expect("Should read length prefix");
    let message_length = u32::from_le_bytes(length_bytes) as usize;
    
    println!("Client expecting message of {} bytes", message_length);
    
    // Read the full message
    buffer.resize(message_length, 0);
    client_stream.read_exact(&mut buffer).expect("Should read full message");
    
    // Reconstruct the full serialized message
    let mut full_message = Vec::new();
    full_message.extend_from_slice(&length_bytes);
    full_message.extend_from_slice(&buffer);
    
    let (message, _) = Message::deserialize(&full_message)
        .expect("Should deserialize large message");
    
    match message {
        Message::AudioData(received_data) => {
            assert_eq!(received_data.len(), large_data.len());
            assert_eq!(received_data, large_data);
            println!("Successfully received and verified large message of {} bytes", received_data.len());
        }
        _ => panic!("Should receive audio data message"),
    }
    
    server_handle.join().expect("Server should complete");
}

/// Test message ordering and sequencing
#[test]
fn test_message_sequencing() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Should bind");
    let port = listener.local_addr().unwrap().port();
    
    let server_handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("Should accept");
        
        // Send messages in specific order
        let messages = vec![
            Message::Config(AudioConfig {
                sample_rate: 48000,
                channels: 1,
                sample_format: SampleFormat::I16,
            }),
            Message::AudioData(vec![1, 2, 3, 4]),
            Message::AudioData(vec![5, 6, 7, 8]),
            Message::Error("End of stream".to_string()),
        ];
        
        for (i, msg) in messages.iter().enumerate() {
            let serialized = msg.serialize().expect("Should serialize message");
            stream.write_all(&serialized).expect("Should send message");
            println!("Server sent message {}: {:?}", i + 1, msg);
            
            // Small delay between messages
            thread::sleep(Duration::from_millis(10));
        }
    });
    
    let mut client_stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .expect("Should connect");
    
    // Read all messages in sequence
    let mut message_buffer = Vec::new();
    let mut received_messages = Vec::new();
    
    // Read all data
    let mut read_buffer = vec![0u8; 4096];
    while received_messages.len() < 4 {
        match client_stream.read(&mut read_buffer) {
            Ok(0) => break, // Connection closed
            Ok(bytes_read) => {
                message_buffer.extend_from_slice(&read_buffer[..bytes_read]);
                
                // Try to deserialize messages
                while let Ok((message, remaining)) = Message::deserialize(&message_buffer) {
                    received_messages.push(message);
                    message_buffer = remaining.to_vec();
                    
                    if message_buffer.len() < 4 {
                        break; // Not enough for next message
                    }
                }
            }
            Err(_) => break,
        }
    }
    
    println!("Client received {} messages", received_messages.len());
    
    // Verify message sequence
    assert_eq!(received_messages.len(), 4, "Should receive 4 messages");
    
    match &received_messages[0] {
        Message::Config(config) => {
            assert_eq!(config.sample_rate, 48000);
            assert_eq!(config.channels, 1);
            assert_eq!(config.sample_format, SampleFormat::I16);
        }
        _ => panic!("First message should be Config"),
    }
    
    match &received_messages[1] {
        Message::AudioData(data) => assert_eq!(data, &vec![1, 2, 3, 4]),
        _ => panic!("Second message should be AudioData"),
    }
    
    match &received_messages[2] {
        Message::AudioData(data) => assert_eq!(data, &vec![5, 6, 7, 8]),
        _ => panic!("Third message should be AudioData"),
    }
    
    match &received_messages[3] {
        Message::Error(error) => assert_eq!(error, "End of stream"),
        _ => panic!("Fourth message should be Error"),
    }
    
    println!("All messages received in correct sequence!");
    
    server_handle.join().expect("Server should complete");
}

/// Test the default port constant
#[test]
fn test_default_port() {
    assert_eq!(DEFAULT_PORT, 8080);
}