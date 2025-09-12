use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::io::Read;
use std::net::TcpStream;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
use strim_shared::DEFAULT_PORT;

mod cli_commands;
use clap::Parser;

fn main() -> Result<()> {
    // let args = cli_commands::Cli::parse();
    
    // println!("Connecting to server at {}:{}", args.host, args.port);
    
    let stream = TcpStream::connect(("0.0.0.0", DEFAULT_PORT))?;
    stream.set_nodelay(true)?;
    
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    
    // Spawn thread to read from network
    let stream_clone = stream.try_clone()?;
    thread::spawn(move || network_read_loop(stream_clone, tx));
    
    // Start audio playback
    start_audio_playback(rx)?;
    
    Ok(())
}

fn network_read_loop(mut stream: TcpStream, tx: mpsc::Sender<Vec<u8>>) {
    let mut buffer = [0u8; 4096];
    
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => {
                println!("Server disconnected");
                break;
            }
            Ok(n) => {
                let data = buffer[..n].to_vec();
                if tx.send(data).is_err() {
                    break;
                }
            }
            Err(e) => {
                eprintln!("Network read error: {e}");
                break;
            }
        }
    }
}

fn start_audio_playback(rx: mpsc::Receiver<Vec<u8>>) -> Result<()> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("No default output device"))?;

    let supported_config = device
        .default_output_config()
        .map_err(|e| anyhow::anyhow!("Failed to get default output config: {e}"))?;

    println!("Playbak config: {:?}", &supported_config);

    let sample_format = supported_config.sample_format();
    let config: StreamConfig = supported_config.into();

    let err_fn = |err| eprintln!("Audio playback error: {err}");

    // Create a buffer to hold incoming audio data
    let audio_buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let buffer_for_network = Arc::clone(&audio_buffer);
    let buffer_for_audio = Arc::clone(&audio_buffer);

    // Spawn thread to receive network data
    thread::spawn(move || {
        while let Ok(data) = rx.recv() {
            let mut buffer = buffer_for_network.lock().unwrap();
            buffer.extend_from_slice(&data);
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
    
    // Keep the main thread alive
    loop {
        thread::sleep(Duration::from_secs(1));
    }
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
