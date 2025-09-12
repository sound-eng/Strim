use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::io::Read;
use std::net::TcpStream;
use std::sync::{mpsc, Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::Duration;
use strim_shared::{Message, AudioConfig, SampleFormat as SharedSampleFormat};

mod cli_commands;
use clap::Parser;

fn main() -> Result<()> {
    let args = cli_commands::Cli::parse();
    
    // Set up graceful shutdown handling
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);
    
    ctrlc::set_handler(move || {
        println!("\nReceived Ctrl+C, shutting down gracefully...");
        running_clone.store(false, Ordering::SeqCst);
    })?;
    
    println!("Connecting to server at {}:{}", args.host, args.port);
    println!("Press Ctrl+C to disconnect gracefully");
    
    let stream = TcpStream::connect((args.host.as_str(), args.port))?;
    stream.set_nodelay(true)?;
    
    let (tx, rx) = mpsc::channel::<Message>();
    
    // Spawn thread to read from network
    let stream_clone = stream.try_clone()?;
    let running_network = Arc::clone(&running);
    thread::spawn(move || network_read_loop(stream_clone, tx, running_network));
    
    // Start audio playback
    start_audio_playback(rx, running)?;
    
    println!("Client disconnected");
    Ok(())
}

/// Read messages from the network and deserialize them
/// Sends deserialized messages to the audio playback thread
fn network_read_loop(mut stream: TcpStream, tx: mpsc::Sender<Message>, running: Arc<AtomicBool>) {
    let mut buffer = [0u8; 4096];
    
    while running.load(Ordering::SeqCst) {
        match stream.read(&mut buffer) {
            Ok(0) => {
                println!("Server disconnected");
                break;
            }
            Ok(n) => {
                let data = buffer[..n].to_vec();
                // Try to deserialize the message
                match Message::deserialize(&data) {
                    Ok(message) => {
                        if tx.send(message).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to deserialize message: {e}");
                        // Continue reading in case of partial data
                    }
                }
            }
            Err(e) => {
                if running.load(Ordering::SeqCst) {
                    eprintln!("Network read error: {e}");
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
    let audio_config = match rx.recv() {
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
        while running_audio.load(Ordering::SeqCst) {
            match rx.recv() {
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
                Err(_) => {
                    // Channel closed, exit gracefully
                    break;
                }
            }
        }
    });

    

    let stream = match sample_format {
        SampleFormat::F32 => device.build_output_stream(
            &config,
            move |data: &mut [f32], _| on_output_data_f32(data, &buffer_for_audio),
            err_fn,
            None,
        )?,
        SampleFormat::I16 => device.build_output_stream(
            &config,
            move |data: &mut [i16], _| on_output_data_i16(data, &buffer_for_audio),
            err_fn,
            None,
        )?,
        SampleFormat::U16 => device.build_output_stream(
            &config,
            move |data: &mut [u16], _| on_output_data_u16(data, &buffer_for_audio),
            err_fn,
            None,
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

fn on_output_data_f32(data: &mut [f32], buffer: &Arc<Mutex<Vec<u8>>>) {
    let mut audio_buffer = buffer.lock().unwrap();
    let bytes_needed = data.len() * 4;
    
    if audio_buffer.len() < bytes_needed {
        // Not enough data, fill with silence
        data.fill(0.0);
        return;
    }
    
    // Convert bytes to f32 samples
    for (i, sample_bytes) in audio_buffer.drain(..bytes_needed).collect::<Vec<_>>().chunks(4).enumerate() {
        if i < data.len() {
            data[i] = f32::from_le_bytes([sample_bytes[0], sample_bytes[1], sample_bytes[2], sample_bytes[3]]);
        }
    }
}

fn on_output_data_i16(data: &mut [i16], buffer: &Arc<Mutex<Vec<u8>>>) {
    let mut audio_buffer = buffer.lock().unwrap();
    let bytes_needed = data.len() * 2;
    
    if audio_buffer.len() < bytes_needed {
        // Not enough data, fill with silence
        data.fill(0);
        return;
    }
    
    // Convert bytes to i16 samples
    for (i, sample_bytes) in audio_buffer.drain(..bytes_needed).collect::<Vec<_>>().chunks(2).enumerate() {
        if i < data.len() {
            data[i] = i16::from_le_bytes([sample_bytes[0], sample_bytes[1]]);
        }
    }
}

fn on_output_data_u16(data: &mut [u16], buffer: &Arc<Mutex<Vec<u8>>>) {
    let mut audio_buffer = buffer.lock().unwrap();
    let bytes_needed = data.len() * 2;
    
    if audio_buffer.len() < bytes_needed {
        // Not enough data, fill with silence
        data.fill(0);
        return;
    }
    
    // Convert bytes to u16 samples
    for (i, sample_bytes) in audio_buffer.drain(..bytes_needed).collect::<Vec<_>>().chunks(2).enumerate() {
        if i < data.len() {
            data[i] = u16::from_le_bytes([sample_bytes[0], sample_bytes[1]]);
        }
    }
}
