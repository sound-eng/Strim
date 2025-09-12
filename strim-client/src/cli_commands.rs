use clap::Parser;

/// Audio streaming client
#[derive(Parser)]
#[command(name = "strim-client", version, about = "Audio streaming client")]
pub struct Cli {
    #[arg(short, long, default_value = "localhost")]
    pub host: String,
    
    #[arg(short, long, default_value = "8080")]
    pub port: u16,
}
