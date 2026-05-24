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
    Validate {
        file: String,
        #[arg(long)]
        short: bool,
    },
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
        Commands::Validate { file, short } => {
            std::process::exit(run_validate(file, *short));
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

fn run_validate(file: &str, short: bool) -> i32 {
    let source = match read_source_file(file) {
        Ok(s) => s,
        Err(e) => {
            println!("{file}:1:1: error: cannot read file: {e}");
            return 1;
        }
    };

    let sm = otter_fusion::source_map::SourceMap::new(file, &source);

    let tokens = match Lexer::new(&source).scan_all() {
        Ok(t) => t,
        Err(e) => {
            let (line, col) = e.span();
            if short {
                println!("{file}:{line}:{col}: error: {e}");
            } else {
                print!("{}", sm.render_error(line, col, &format!("{e}")));
            }
            return 1;
        }
    };

    let program = match otter_fusion::parser::Parser::new(tokens).parse() {
        Ok(p) => p,
        Err(e) => {
            let (line, col) = e.span();
            if short {
                println!("{file}:{line}:{col}: error: {e}");
            } else {
                print!("{}", sm.render_error(line, col, &format!("{e}")));
            }
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

    let modules = vec![
        otter_fusion::get_core_module(),
        module,
    ];

    match Validator::new(modules).validate() {
        Ok(_) => 0,
        Err(errors) => {
            use otter_fusion::validator::ValidationError;
            for err in &errors {
                let (line, col) = err.span();
                
                // Extract the keyword/lexeme associated with the error
                let keyword = match err {
                    ValidationError::UnknownVariable { name, .. } => name.as_str(),
                    ValidationError::TypeMismatch { context, .. } => {
                        if context.starts_with("var ") {
                            context.strip_prefix("var ").unwrap_or("")
                        } else {
                            ""
                        }
                    }
                    ValidationError::UnknownMember { member, .. } => member.as_str(),
                    ValidationError::UnknownImportModule { target, .. } => target.as_str(),
                    ValidationError::UnknownImportSymbol { symbol, .. } => symbol.as_str(),
                    _ => "",
                };

                let (refined_line, refined_col, _len) = sm.find_keyword_span(line, col, keyword);

                if short {
                    println!("{file}:{refined_line}:{refined_col}: error: {err}");
                } else {
                    print!("{}", sm.render_error(refined_line, refined_col, &format!("{err}")));
                }
            }
            1
        }
    }
}
