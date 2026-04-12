use clap::{Parser, Subcommand};

#[derive(Subcommand)]
enum Commands {
    Scan { file: String },
    Parse { file: String },
    Validate { file: String },
    Run { file: String },
    Compile { file: String },
}
#[derive(Parser)]
#[command(version, about)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Scan { file } => {
            println!("Scanning: {file}");
        }
        Commands::Parse { file } => {
            println!("Parsing: {file}");
        }
        Commands::Validate { file } => {
            println!("Validating: {file}");
        }
        Commands::Run { file } => {
            println!("Running: {file}");
        }
        Commands::Compile { file } => {
            println!("Compiling: {file}");
        }
    }
}
