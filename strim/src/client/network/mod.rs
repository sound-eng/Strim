pub mod protocol;
pub mod connection;

use anyhow::Result;

pub fn run_client(host: String, port: u16) -> Result<()> {
    protocol::start(host, port)
}

