pub mod audio;
pub mod network;

use anyhow::Result;

pub fn run(host: String, port: u16) -> Result<()> {
    network::run_client(host, port)
}

