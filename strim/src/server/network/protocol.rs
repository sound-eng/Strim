use anyhow::Result;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::Duration;

use strim_shared::Message;

use crate::server::audio::capture::{get_audio_config, start_default_input_capture};
use crate::server::network::connection::get_local_ip;

pub fn start(port: u16) -> Result<()> {
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);
    let running_accept = Arc::clone(&running);
    let running_broadcast = Arc::clone(&running);
    let running_health = Arc::clone(&running);

    ctrlc::set_handler(move || {
        println!("\nReceived Ctrl+C, shutting down server gracefully...");
        running_clone.store(false, Ordering::SeqCst);
    })?;

    let (tx, rx) = mpsc::channel::<Message>();
    let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
    let clients_for_accept = Arc::clone(&clients);
    let clients_for_config = Arc::clone(&clients);
    let clients_for_shutdown = Arc::clone(&clients);
    let clients_for_health = Arc::clone(&clients);

    let audio_config = get_audio_config()?;

    thread::spawn(move || accept_loop(clients_for_accept, audio_config, port, running_accept));
    thread::spawn(move || broadcast_loop(rx, clients_for_config, running_broadcast));
    thread::spawn(move || health_check_loop(clients_for_health, running_health));

    let _stream = start_default_input_capture(tx)?;

    if let Some(ip) = get_local_ip() { println!("Server running. Local IP: {}", ip); } else { println!("Server running."); }
    while running.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(100));
    }

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

    drop(_stream);
    println!("Audio capture stopped");
    println!("Server shutdown complete");
    Ok(())
}

fn accept_loop(clients: Arc<Mutex<Vec<TcpStream>>>, audio_config: strim_shared::AudioConfig, port: u16, running: Arc<AtomicBool>) {
    let listener = match TcpListener::bind(("0.0.0.0", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind TCP listener: {e}");
            return;
        }
    };
    println!("TCP server listening on 0.0.0.0:{}", port);
    listener.set_nonblocking(true).expect("Failed to set non-blocking");

    while running.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, addr)) => {
                let _ = stream.set_nodelay(true);
                println!("Client connected: {:?}", addr);
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
                clients.lock().unwrap().push(stream);
                let client_count = clients.lock().unwrap().len();
                println!("Total clients connected: {}", client_count);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
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

fn broadcast_loop(rx: mpsc::Receiver<Message>, clients: Arc<Mutex<Vec<TcpStream>>>, running: Arc<AtomicBool>) {
    while running.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(message) => {
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
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    println!("Broadcast loop stopped");
}

fn health_check_loop(clients: Arc<Mutex<Vec<TcpStream>>>, running: Arc<AtomicBool>) {
    while running.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_secs(5));
        if !running.load(Ordering::SeqCst) { break; }

        let mut lock = match clients.lock() {
            Ok(lock) => lock,
            Err(_) => { eprintln!("Mutex poisoned, skipping health check"); continue; }
        };
        let mut i = 0;
        while i < lock.len() {
            let client_addr = lock[i].peer_addr().map(|addr| addr.to_string()).unwrap_or_else(|_| "unknown".to_string());
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
                println!("Client removed (serialization error): {:?}", client_addr);
                let _ = lock[i].shutdown(std::net::Shutdown::Both);
                lock.remove(i);
            }
        }
    }
    println!("Health check loop stopped");
}

