use clap::{Parser, Subcommand};
use otter_fusion::{
    ast::Module,
    lexer::Lexer,
    validator::Validator,
};

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
            std::process::exit(run_validate(file));
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

fn run_validate(file: &str) -> i32 {
    let source = match read_source_file(file) {
        Ok(s) => s,
        Err(e) => {
            println!("{file}:1:1: error: cannot read file: {e}");
            return 1;
        }
    };

    let tokens = match Lexer::new(&source).scan_all() {
        Ok(t) => t,
        Err(e) => {
            let (line, col) = e.span();
            println!("{file}:{line}:{col}: error: {e}");
            return 1;
        }
    };

    let program = match otter_fusion::parser::Parser::new(tokens).parse() {
        Ok(p) => p,
        Err(e) => {
            let (line, col) = e.span();
            println!("{file}:{line}:{col}: error: {e}");
            return 1;
        }
    };

    let module_name = std::path::Path::new(file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("main")
        .to_string();

    let module = Module {
        name: module_name,
        program,
    };

    match Validator::new(vec![module]).validate() {
        Ok(_) => 0,
        Err(errors) => {
            for err in &errors {
                println!("{file}:1:1: error: {err}");
            }
            1
        }
    }
}
