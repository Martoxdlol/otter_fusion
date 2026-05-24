use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use cranelift_codegen::settings::Configurable;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::default_libcall_names;
use cranelift_object::{ObjectBuilder, ObjectModule};
use otter_fusion::{
    ast::Module,
    codegen,
    hir::{Hir, PrimitiveType},
    lexer::Lexer,
    lower::Lower,
    mir::{MirProgram, MirType},
    validator::Validator,
};

#[derive(Subcommand)]
enum Commands {
    Scan { file: String },
    Parse { file: String },
    Validate { file: String },
    Run { file: String },
    Compile {
        file: String,
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

    let code = match &cli.command {
        Commands::Scan { file } => run_scan(file),
        Commands::Parse { file } => run_parse(file),
        Commands::Validate { file } => run_validate(file),
        Commands::Run { file } => run_run(file),
        Commands::Compile { file, output } => run_compile(file, output.as_deref()),
    };
    std::process::exit(code);
}

fn run_scan(file: &str) -> i32 {
    let source = match read_source_file(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{file}:1:1: error: cannot read file: {e}");
            return 1;
        }
    };
    let tokens = match Lexer::new(&source).scan_all() {
        Ok(t) => t,
        Err(e) => {
            let (line, col) = e.span();
            eprintln!("{file}:{line}:{col}: error: {e}");
            return 1;
        }
    };
    println!("{tokens:#?}");
    0
}

fn run_parse(file: &str) -> i32 {
    let source = match read_source_file(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{file}:1:1: error: cannot read file: {e}");
            return 1;
        }
    };
    let tokens = match Lexer::new(&source).scan_all() {
        Ok(t) => t,
        Err(e) => {
            let (line, col) = e.span();
            eprintln!("{file}:{line}:{col}: error: {e}");
            return 1;
        }
    };
    match otter_fusion::parser::Parser::new(tokens).parse() {
        Ok(ast) => {
            println!("{ast:#?}");
            0
        }
        Err(e) => {
            let (line, col) = e.span();
            eprintln!("{file}:{line}:{col}: error: {e}");
            1
        }
    }
}

fn run_validate(file: &str) -> i32 {
    match build_hir(file) {
        Ok(_) => 0,
        Err(code) => code,
    }
}

fn run_run(file: &str) -> i32 {
    let mir = match build_mir(file) {
        Ok(m) => m,
        Err(code) => return code,
    };

    let entry = mir.functions.get(&mir.entry).expect("entry function");
    if !entry.params.is_empty() {
        eprintln!("{file}: error: `main` must take no arguments to be run via JIT");
        return 1;
    }

    let jit = match make_jit_module() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{file}: error: failed to build JIT: {e}");
            return 1;
        }
    };

    let mut compiled = match codegen::compile(&mir, jit) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{file}: error: codegen: {e:#?}");
            return 1;
        }
    };
    if let Err(e) = compiled.module.finalize_definitions() {
        eprintln!("{file}: error: finalize: {e}");
        return 1;
    }

    let func_id = compiled.function_ids[&mir.entry];
    let code_ptr = compiled.module.get_finalized_function(func_id);

    match &entry.return_type {
        MirType::Unit => {
            let f: extern "C" fn() = unsafe { std::mem::transmute(code_ptr) };
            f();
            0
        }
        MirType::Primitive(PrimitiveType::Int64) => {
            let f: extern "C" fn() -> i64 = unsafe { std::mem::transmute(code_ptr) };
            let result = f();
            println!("{result}");
            0
        }
        MirType::Primitive(PrimitiveType::Int32) => {
            let f: extern "C" fn() -> i32 = unsafe { std::mem::transmute(code_ptr) };
            println!("{}", f());
            0
        }
        MirType::Primitive(PrimitiveType::Bool) => {
            let f: extern "C" fn() -> i8 = unsafe { std::mem::transmute(code_ptr) };
            println!("{}", f() != 0);
            0
        }
        other => {
            eprintln!("{file}: error: unsupported `main` return type for JIT: {other:?}");
            1
        }
    }
}

fn run_compile(file: &str, output: Option<&str>) -> i32 {
    let mir = match build_mir(file) {
        Ok(m) => m,
        Err(code) => return code,
    };

    let isa_builder = match cranelift_native::builder() {
        Ok(b) => b,
        Err(msg) => {
            eprintln!("{file}: error: host machine not supported: {msg}");
            return 1;
        }
    };
    let flags = cranelift_codegen::settings::Flags::new(cranelift_codegen::settings::builder());
    let isa = match isa_builder.finish(flags) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("{file}: error: isa: {e}");
            return 1;
        }
    };
    let obj_name = Path::new(file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("main")
        .to_string();
    let builder = match ObjectBuilder::new(isa, obj_name.clone(), default_libcall_names()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{file}: error: object builder: {e}");
            return 1;
        }
    };
    let object_module = ObjectModule::new(builder);

    let compiled = match codegen::compile(&mir, object_module) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{file}: error: codegen: {e}");
            return 1;
        }
    };
    let product = compiled.module.finish();
    let bytes = match product.emit() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{file}: error: emit object: {e}");
            return 1;
        }
    };

    let out_path = output
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(file).with_extension("o"));
    if let Err(e) = std::fs::write(&out_path, &bytes) {
        eprintln!("{}: error: write object: {e}", out_path.display());
        return 1;
    }
    println!("{}", out_path.display());
    0
}

fn build_hir(file: &str) -> Result<Hir, i32> {
    let source = read_source_file(file).map_err(|e| {
        eprintln!("{file}:1:1: error: cannot read file: {e}");
        1
    })?;
    let tokens = Lexer::new(&source).scan_all().map_err(|e| {
        let (line, col) = e.span();
        eprintln!("{file}:{line}:{col}: error: {e}");
        1
    })?;
    let program = otter_fusion::parser::Parser::new(tokens)
        .parse()
        .map_err(|e| {
            let (line, col) = e.span();
            eprintln!("{file}:{line}:{col}: error: {e}");
            1
        })?;
    let module_name = Path::new(file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("main")
        .to_string();
    let module = Module {
        name: module_name,
        program,
    };
    Validator::new(vec![module]).validate().map_err(|errors| {
        for err in &errors {
            eprintln!("{file}:1:1: error: {err}");
        }
        1
    })
}

// cranelift-jit 0.110 hardcodes `is_pic=true` in JITBuilder::new, which makes
// its PLT writer panic on non-x86_64. Build the ISA ourselves with is_pic off.
fn make_jit_module() -> Result<JITModule, String> {
    let mut flags = cranelift_codegen::settings::builder();
    flags.set("use_colocated_libcalls", "false").map_err(|e| e.to_string())?;
    flags.set("is_pic", "false").map_err(|e| e.to_string())?;
    let isa_builder = cranelift_native::builder().map_err(|m| m.to_string())?;
    let isa = isa_builder
        .finish(cranelift_codegen::settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    let mut builder = JITBuilder::with_isa(isa, default_libcall_names());
    register_runtime_shims(&mut builder);
    Ok(JITModule::new(builder))
}

// Minimal runtime stubs so JIT smoke tests resolve __of_* symbols. Real
// programs link a proper runtime; this is just enough to run examples.
fn register_runtime_shims(builder: &mut JITBuilder) {
    builder.symbol("__of_alloc", rt::of_alloc as *const u8);
    builder.symbol("__of_vtable_lookup", rt::of_vtable_lookup as *const u8);
    builder.symbol("__of_print", rt::of_print as *const u8);
    builder.symbol("__of_println", rt::of_println as *const u8);
    builder.symbol("__of_str_concat", rt::of_str_concat as *const u8);
}

mod rt {
    use std::ffi::CStr;
    use std::os::raw::{c_char, c_void};

    pub unsafe extern "C" fn of_alloc(size: u64, _type_id: u64) -> *mut c_void {
        let total = (size.max(16) + 16) as usize;
        let layout = std::alloc::Layout::from_size_align(total, 8).unwrap();
        unsafe {
            let ptr = std::alloc::alloc_zeroed(layout);
            // Skip a fake header (16 bytes) so callers see a "body" pointer.
            ptr.add(16) as *mut c_void
        }
    }

    pub unsafe extern "C" fn of_vtable_lookup(
        _recv: *const c_void,
        _iface_id: u64,
        _slot: u64,
    ) -> *const c_void {
        // Unimplemented runtime: no real vtable resolution. Programs that
        // hit a virtual call will crash; non-virtual code paths still run.
        std::ptr::null()
    }

    pub unsafe extern "C" fn of_print(s: *const c_char) {
        if s.is_null() {
            return;
        }
        let cstr = unsafe { CStr::from_ptr(s) };
        print!("{}", cstr.to_string_lossy());
    }

    pub unsafe extern "C" fn of_println(s: *const c_char) {
        unsafe { of_print(s) };
        println!();
    }

    pub unsafe extern "C" fn of_str_concat(
        a: *const c_char,
        b: *const c_char,
    ) -> *const c_char {
        let a = if a.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(a) }.to_string_lossy().into_owned()
        };
        let b = if b.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(b) }.to_string_lossy().into_owned()
        };
        let combined = format!("{}{}\0", a, b);
        let boxed = combined.into_bytes().into_boxed_slice();
        Box::leak(boxed).as_ptr() as *const c_char
    }
}

fn build_mir(file: &str) -> Result<MirProgram, i32> {
    let hir = build_hir(file)?;
    Lower::new(hir).lower().map_err(|e| {
        eprintln!("{file}: error: lower: {e}");
        1
    })
}
