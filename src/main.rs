use clap::{Parser, Subcommand};
use otter_fusion::{
    ast::Module,
    codegen::{Codegen, ENTRY_SYMBOL},
    lexer::Lexer,
    lower::Lower,
    mir::MirProgram,
    validator::Validator,
};

#[derive(Subcommand)]
enum Commands {
    Scan {
        file: String,
    },
    Parse {
        file: String,
    },
    Validate {
        file: String,
    },
    /// JIT-compile the program and run it in-process.
    Run {
        file: String,
    },
    /// AOT-compile the program to a relocatable object file. Link with
    /// libotter_runtime.a (built from the otter_runtime crate) to
    /// produce an executable.
    Compile {
        file: String,
        /// Output path. Defaults to `<stem>.o` next to the source.
        #[arg(short, long)]
        output: Option<String>,
    },
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
            std::process::exit(run_jit(file));
        }
        Commands::Compile { file, output } => {
            std::process::exit(run_compile(file, output.as_deref()));
        }
    }

    Ok(())
}

/// Runs the front-end (read → lex → parse → validate → lower) and
/// returns a `MirProgram` ready for codegen, or an exit code.
fn frontend(file: &str) -> Result<MirProgram, i32> {
    let source = match read_source_file(file) {
        Ok(s) => s,
        Err(e) => {
            println!("{file}:1:1: error: cannot read file: {e}");
            return Err(1);
        }
    };

    let tokens = match Lexer::new(&source).scan_all() {
        Ok(t) => t,
        Err(e) => {
            let (line, col) = e.span();
            println!("{file}:{line}:{col}: error: {e}");
            return Err(1);
        }
    };

    let program = match otter_fusion::parser::Parser::new(tokens).parse() {
        Ok(p) => p,
        Err(e) => {
            let (line, col) = e.span();
            println!("{file}:{line}:{col}: error: {e}");
            return Err(1);
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

    let hir = match Validator::new(vec![module]).validate() {
        Ok(h) => h,
        Err(errors) => {
            for err in &errors {
                println!("{file}:1:1: error: {err}");
            }
            return Err(1);
        }
    };

    Lower::new(hir).lower().map_err(|e| {
        println!("{file}:1:1: error: lower: {e}");
        1
    })
}

fn run_validate(file: &str) -> i32 {
    match frontend(file) {
        Ok(_) => 0,
        Err(code) => code,
    }
}

/// Symbol table the JIT uses to resolve unresolved imports. The
/// `otter_runtime` crate is linked into this binary as an `rlib`, so we
/// can hand its function addresses straight to the JIT.
fn jit_symbol_table() -> Vec<(&'static str, *const u8)> {
    vec![
        (
            "otter_alloc_struct",
            otter_runtime::otter_alloc_struct as *const u8,
        ),
        (
            "otter_alloc_env",
            otter_runtime::otter_alloc_env as *const u8,
        ),
        (
            "otter_alloc_closure",
            otter_runtime::otter_alloc_closure as *const u8,
        ),
        (
            "otter_union_construct",
            otter_runtime::otter_union_construct as *const u8,
        ),
        (
            "otter_union_tag",
            otter_runtime::otter_union_tag as *const u8,
        ),
        (
            "otter_union_payload",
            otter_runtime::otter_union_payload as *const u8,
        ),
        (
            "otter_vcall_lookup",
            otter_runtime::otter_vcall_lookup as *const u8,
        ),
        ("otter_trap", otter_runtime::otter_trap as *const u8),
    ]
}

fn run_jit(file: &str) -> i32 {
    let program = match frontend(file) {
        Ok(p) => p,
        Err(code) => return code,
    };

    let mut cg = match Codegen::jit(&jit_symbol_table()) {
        Ok(cg) => cg,
        Err(e) => {
            eprintln!("codegen: {e}");
            return 1;
        }
    };
    if let Err(e) = cg.compile(&program) {
        eprintln!("codegen: {e}");
        return 1;
    }
    let entry_ptr = match cg.finish(program.entry) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("codegen: {e}");
            return 1;
        }
    };

    // The entry function returns whatever Lower interned for `main`'s
    // declared return type. For now we read it back as i64 — i8/i32
    // results sit in the low bits and round-trip correctly through the
    // C ABI on every supported target.
    let entry: extern "C" fn() -> i64 = unsafe { std::mem::transmute(entry_ptr) };
    entry() as i32
}

fn run_compile(file: &str, output: Option<&str>) -> i32 {
    let program = match frontend(file) {
        Ok(p) => p,
        Err(code) => return code,
    };

    let stem = std::path::Path::new(file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("program")
        .to_string();

    let mut cg = match Codegen::object(&stem) {
        Ok(cg) => cg,
        Err(e) => {
            eprintln!("codegen: {e}");
            return 1;
        }
    };
    if let Err(e) = cg.compile(&program) {
        eprintln!("codegen: {e}");
        return 1;
    }
    let bytes = match cg.finish() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("codegen: {e}");
            return 1;
        }
    };

    let out_path = output.map(|s| s.to_string()).unwrap_or_else(|| {
        std::path::Path::new(file)
            .with_extension("o")
            .to_string_lossy()
            .into_owned()
    });

    if let Err(e) = std::fs::write(&out_path, &bytes) {
        eprintln!("write {out_path}: {e}");
        return 1;
    }

    println!("wrote {out_path} ({} bytes)", bytes.len());
    println!("entry symbol: {ENTRY_SYMBOL}");
    println!(
        "link example: cc {out_path} -L<dir-with-libotter_runtime.a> -lotter_runtime -o {stem}"
    );
    0
}
