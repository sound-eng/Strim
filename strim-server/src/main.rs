use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::fmt::Debug;
use std::time::Duration;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use strim_shared::{DEFAULT_PORT, Message, AudioConfig, SampleFormat as SharedSampleFormat};

mod cli_commands;
mod capture;

fn main() {
    let (tx, rx) = mpsc::channel::<Message>();

    let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
    let clients_for_accept = Arc::clone(&clients);
    let clients_for_config = Arc::clone(&clients);
    
    // Get audio config first
    let audio_config = match get_audio_config() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("Error getting audio config: {err}");
            return;
        }
    };

    thread::spawn(move || accept_loop(clients_for_accept, audio_config));
    thread::spawn(move || broadcast_loop(rx, clients_for_config));

    let _stream = match start_default_input_capture(tx) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("Error starting capture: {err}");
            return;
        }
    };

    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
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
fn accept_loop(clients: Arc<Mutex<Vec<TcpStream>>>, audio_config: AudioConfig) {
    let listener = match TcpListener::bind(("0.0.0.0", DEFAULT_PORT)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind TCP listener: {e}");
            return;
        }
    };
    println!("TCP server listening on 0.0.0.0:{}", DEFAULT_PORT);
    for conn in listener.incoming() {
        match conn {
            Ok(mut stream) => {
                let _ = stream.set_nodelay(true);
                println!("Client connected: {:?}", stream.peer_addr());
                
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
            }
            Err(e) => eprintln!("Accept error: {e}"),
        }
    }
}

/// Broadcast messages to all connected clients
/// Serializes messages and sends them over TCP, removing disconnected clients
fn broadcast_loop(rx: mpsc::Receiver<Message>, clients: Arc<Mutex<Vec<TcpStream>>>) {
    while let Ok(message) = rx.recv() {
        // Serialize the message to bytes
        let serialized = match message.serialize() {
            Ok(data) => data,
            Err(e) => {
                eprintln!("Failed to serialize message: {e}");
                continue;
            }
        };

        let mut lock = clients.lock().unwrap();
        let mut i = 0;
        while i < lock.len() {
            let write_res = lock[i].write_all(&serialized);
            if write_res.is_err() {
                let _ = lock[i].shutdown(std::net::Shutdown::Both);
                lock.remove(i);
            } else {
                i += 1;
            }
        }
    }
}
