use clap::{Parser, Subcommand};
use otter_fusion::lexer::Lexer;

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

fn read_source_file(file: &str) -> Result<String, std::io::Error> {
    std::fs::read_to_string(file)
}

fn main() -> Result<(), std::io::Error> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Scan { file } => {
            let source = read_source_file(file)?;
            let mut scanner = Lexer::new(&source);
            let tokens = scanner.scan_all().expect("Failed to scan tokens");
            println!("{tokens:#?}");
        }
        Commands::Parse { file } => {
            let source = read_source_file(file)?;
            let mut scanner = Lexer::new(&source);
            let tokens = scanner.scan_all().expect("Failed to scan tokens");
            let mut parser = otter_fusion::parser::Parser::new(tokens);
            let ast = parser.parse().expect("Failed to parse source code");
            println!("{ast:#?}");
        }
        Commands::Validate { file } => {
            let source = read_source_file(file)?;
            let mut scanner = Lexer::new(&source);
            let tokens = scanner.scan_all().expect("Failed to scan tokens");
            let mut parser = otter_fusion::parser::Parser::new(tokens);
            let ast = parser.parse().expect("Failed to parse source code");
            let module = otter_fusion::ast::Module {
                name: file.clone(),
                program: ast,
            };
            let validator = otter_fusion::validator::Validator::new(vec![module]);
            match validator.validate() {
                Ok(hir) => println!("{hir:#?}"),
                Err(errors) => {
                    for e in &errors {
                        eprintln!("{e}");
                    }
                    std::process::exit(1);
                }
            }
        }
        Commands::Run { file } => {
            println!("Running: {file}");
        }
        Commands::Compile { file } => {
            println!("Compiling: {file}");
        }
    }

    Ok(())
}
