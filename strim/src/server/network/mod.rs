pub mod protocol;
pub mod connection;

use anyhow::Result;

pub fn run_server(port: u16) -> Result<()> {
    protocol::start(port)
}

