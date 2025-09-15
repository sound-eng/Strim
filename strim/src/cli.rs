use clap::{Parser, Subcommand, Args as ClapArgs};
use strim_shared::DEFAULT_PORT;

#[derive(Parser, Debug)]
#[command(name = "Strim", version, about = "Audio streaming tool")] 
pub struct Args {
    #[command(subcommand)]
    pub mode: Option<Mode>,

    // Used when no subcommand is provided (default server mode)
    #[command(flatten)]
    pub server: ServerArgs,
}

#[derive(Subcommand, Debug)]
pub enum Mode {
    #[command(about = "Run server mode (default)")] 
    Server(ServerArgs),
    #[command(about = "Run client mode")] 
    Client(ClientArgs),
}

#[derive(ClapArgs, Debug)]
pub struct ServerArgs {
    #[arg(short = 'p', long, default_value_t = DEFAULT_PORT, help = "Port to bind server to (for server mode)")]
    pub port: u16,
    #[arg(short = 'd', long, default_value = "", help = "Audio device ID (for server mode)")]
    pub device_id: String,
}

#[derive(ClapArgs, Debug)]
pub struct ClientArgs {
    #[arg(short = 'H', long, default_value = "localhost", help = "Host of the server to connect to (for client mode)")] 
    pub host: String,
    #[arg(short = 'p', long, default_value_t = DEFAULT_PORT, help = "Port on the server to connect to (for client mode)")] 
    pub port: u16,
}

