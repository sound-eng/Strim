use clap::Parser;

/// Interface to parse user commands on the command line
#[derive(Parser)]
#[command(name = "strim-server", version, about = "Audio streaming server")]
pub struct Cli {
    #[arg(short, long, default_value = "8080")]
    pub port: u16,
    
    #[arg(short, long)]
    pub device_id: Option<String>
}

// /// Enum representing all available command line commands in the app
// #[derive(Subcommand)]
// pub enum Commands {
//     Run { device_id: Option<String> },
// }
