use clap::{Parser, Subcommand};

/// Interface to parse user commands on the command line
#[derive(Parser)]
#[command(name = "Strim", version, about = None)]
pub struct Cli {
    #[arg(short, long)]
    pub device_id: String
}

// /// Enum representing all available command line commands in the app
// #[derive(Subcommand)]
// pub enum Commands {
//     Run { device_id: Option<String> },
// }