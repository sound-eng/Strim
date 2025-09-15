use anyhow::Result;
use std::io::Read;
use std::net::TcpStream;
use std::sync::{mpsc, Arc, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::Duration;

use strim_shared::Message;

use crate::client::audio::playback::start_audio_playback;
use crate::client::network::connection::{ConnectionEvent, connect_with_retry};

pub fn start(host: String, port: u16) -> Result<()> {
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);
    
    ctrlc::set_handler(move || {
        println!("\nReceived Ctrl+C, shutting down gracefully...");
        running_clone.store(false, Ordering::SeqCst);
    })?;
    
    println!("Starting audio streaming client");
    println!("Connecting to server at {}:{}", host, port);
    println!("Press Ctrl+C to disconnect gracefully");
    
    // Channel for communication between connection and audio threads
    let (connection_tx, connection_rx) = mpsc::channel::<ConnectionEvent>();
    let (audio_tx, audio_rx) = mpsc::channel::<Message>();
    
    // Spawn connection management thread
    let host_clone = host.clone();
    let running_conn = Arc::clone(&running);
    let audio_tx_clone = audio_tx.clone();
    let connection_tx_clone = connection_tx.clone();
    let initial_connection_handle = thread::spawn(move || {
        connection_manager(host_clone, port, running_conn, connection_tx_clone, audio_tx_clone);
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
                let host_clone = host.clone();
                let running_conn = Arc::clone(&running);
                let audio_tx_clone = audio_tx.clone();
                let connection_tx_clone = connection_tx.clone();
                let handle = thread::spawn(move || {
                    connection_manager(host_clone, port, running_conn, connection_tx_clone, audio_tx_clone);
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

fn connection_manager(host: String, port: u16, running: Arc<AtomicBool>, connection_tx: mpsc::Sender<ConnectionEvent>, audio_tx: mpsc::Sender<Message>) {
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

// connect_with_retry moved to connection.rs

fn network_read_loop(mut stream: TcpStream, tx: mpsc::Sender<Message>, running: Arc<AtomicBool>, connection_tx: mpsc::Sender<ConnectionEvent>) {
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

