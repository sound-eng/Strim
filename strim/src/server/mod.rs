pub mod audio;
pub mod network;

use anyhow::Result;

pub fn run(port: u16, _device_id: String) -> Result<()> {
    network::run_server(port)
}

