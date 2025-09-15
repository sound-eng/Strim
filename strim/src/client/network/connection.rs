use anyhow::Result;
use std::net::TcpStream;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Duration;
use std::thread;

#[derive(Debug)]
pub enum ConnectionEvent { Connected(TcpStream), Disconnected, Shutdown }

pub fn connect_with_retry(host: &str, port: u16, running: Arc<AtomicBool>) -> Result<TcpStream> {
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
