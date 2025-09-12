use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::fmt::Debug;
use std::time::Duration;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use strim_shared::DEFAULT_PORT;

mod cli_commands;
mod capture;

fn main() {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();

    let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
    let clients_for_accept = Arc::clone(&clients);
    thread::spawn(move || accept_loop(clients_for_accept));

    let clients_for_broadcast = Arc::clone(&clients);
    thread::spawn(move || broadcast_loop(rx, clients_for_broadcast));

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

fn start_default_input_capture(tx: mpsc::Sender<Vec<u8>>) -> Result<cpal::Stream> {
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

fn on_input_data_f32(data: &[f32], tx: &mpsc::Sender<Vec<u8>>) {
    let mut out = Vec::with_capacity(data.len() * 4);
    for &s in data {
        out.extend_from_slice(&s.to_le_bytes());
    }
    let _ = tx.send(out);
}

fn on_input_data_i16(data: &[i16], tx: &mpsc::Sender<Vec<u8>>) {
    let mut out = Vec::with_capacity(data.len() * 2);
    for &s in data {
        out.extend_from_slice(&s.to_le_bytes());
    }
    let _ = tx.send(out);
}

fn on_input_data_u16(data: &[u16], tx: &mpsc::Sender<Vec<u8>>) {
    let mut out = Vec::with_capacity(data.len() * 2);
    for &s in data {
        out.extend_from_slice(&s.to_le_bytes());
    }
    let _ = tx.send(out);
}

fn accept_loop(clients: Arc<Mutex<Vec<TcpStream>>>) {
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
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                println!("Client connected: {:?}", stream.peer_addr());
                clients.lock().unwrap().push(stream);
            }
            Err(e) => eprintln!("Accept error: {e}"),
        }
    }
}

fn broadcast_loop(rx: mpsc::Receiver<Vec<u8>>, clients: Arc<Mutex<Vec<TcpStream>>>) {
    while let Ok(chunk) = rx.recv() {
        let mut lock = clients.lock().unwrap();
        let mut i = 0;
        while i < lock.len() {
            let write_res = lock[i].write_all(&chunk);
            if write_res.is_err() {
                let _ = lock[i].shutdown(std::net::Shutdown::Both);
                lock.remove(i);
            } else {
                i += 1;
            }
        }
    }
}
