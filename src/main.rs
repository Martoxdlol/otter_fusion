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
    Validate {
        file: String,
        #[arg(long)]
        short: bool,
    },
    Run {
        file: String,
        // Shared libraries (.dylib / .so) to dlopen and resolve symbols from.
        // Static archives (.a) cannot be loaded at runtime — use `build`.
        #[arg(short = 'l', long = "lib")]
        libs: Vec<String>,
    },
    Compile {
        file: String,
        #[arg(short, long)]
        output: Option<String>,
    },
    Build {
        file: String,
        // Output binary path. Defaults to the source file's stem (no extension).
        #[arg(short, long)]
        output: Option<String>,
        // Extra libraries to link (passed verbatim to cc). Accepts .a, .so, .dylib,
        // or any other path/flag cc understands.
        #[arg(short = 'l', long = "lib")]
        libs: Vec<String>,
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
        Commands::Validate { file, short } => run_validate(file, *short),
        Commands::Run { file, libs } => run_run(file, libs),
        Commands::Compile { file, output } => run_compile(file, output.as_deref()),
        Commands::Build { file, output, libs } => run_build(file, output.as_deref(), libs),
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

fn run_validate(file: &str, short: bool) -> i32 {
    match build_hir(file, short) {
        Ok(_) => 0,
        Err(code) => code,
    }
}

fn run_run(file: &str, lib_paths: &[String]) -> i32 {
    let mir = match build_mir(file) {
        Ok(m) => m,
        Err(code) => return code,
    };

    let entry = mir.functions.get(&mir.entry).expect("entry function");
    if !entry.params.is_empty() {
        eprintln!("{file}: error: `main` must take no arguments to be run via JIT");
        return 1;
    }

    let libs = match load_dynamic_libs(lib_paths) {
        Ok(l) => l,
        Err(code) => return code,
    };

    let jit = match make_jit_module(libs) {
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
    let init_fid = match codegen::emit_vtable_init(&mut compiled, &mir) {
        Ok(fid) => fid,
        Err(e) => {
            eprintln!("{file}: error: emit vtable init: {e}");
            return 1;
        }
    };
    if let Err(e) = compiled.module.finalize_definitions() {
        eprintln!("{file}: error: finalize: {e}");
        return 1;
    }

    let init_ptr = compiled.module.get_finalized_function(init_fid);
    let init: extern "C" fn() = unsafe { std::mem::transmute(init_ptr) };
    init();

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
    let out_path = output
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(file).with_extension("o"));
    if let Err(code) = emit_object(file, &out_path) {
        return code;
    }
    println!("{}", out_path.display());
    0
}

fn emit_object(file: &str, out_path: &Path) -> Result<(), i32> {
    let mir = build_mir(file)?;

    let isa_builder = cranelift_native::builder().map_err(|msg| {
        eprintln!("{file}: error: host machine not supported: {msg}");
        1
    })?;
    // macOS arm64 (and modern Linux) refuse to link non-PIC code into
    // executables. Default cranelift settings have is_pic=false; flip it.
    let mut flag_builder = cranelift_codegen::settings::builder();
    flag_builder.set("is_pic", "true").map_err(|e| {
        eprintln!("{file}: error: isa flag: {e}");
        1
    })?;
    let flags = cranelift_codegen::settings::Flags::new(flag_builder);
    let isa = isa_builder.finish(flags).map_err(|e| {
        eprintln!("{file}: error: isa: {e}");
        1
    })?;
    let obj_name = Path::new(file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("main")
        .to_string();
    let builder = ObjectBuilder::new(isa, obj_name, default_libcall_names()).map_err(|e| {
        eprintln!("{file}: error: object builder: {e}");
        1
    })?;
    let object_module = ObjectModule::new(builder);

    let mut compiled = codegen::compile(&mir, object_module).map_err(|e| {
        eprintln!("{file}: error: codegen: {e}");
        1
    })?;
    codegen::emit_c_main(&mut compiled, &mir).map_err(|e| {
        eprintln!("{file}: error: emit main: {e}");
        1
    })?;
    let product = compiled.module.finish();
    let bytes = product.emit().map_err(|e| {
        eprintln!("{file}: error: emit object: {e}");
        1
    })?;

    std::fs::write(out_path, &bytes).map_err(|e| {
        eprintln!("{}: error: write object: {e}", out_path.display());
        1
    })?;
    Ok(())
}

fn run_build(file: &str, output: Option<&str>, libs: &[String]) -> i32 {
    let rt_lib = match find_otter_rt() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{file}: error: {e}");
            return 1;
        }
    };

    let stem = Path::new(file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("main")
        .to_string();
    let bin_path = output
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&stem));

    let tmp_obj =
        std::env::temp_dir().join(format!("otter_fusion.{}.{stem}.o", std::process::id()));
    if let Err(code) = emit_object(file, &tmp_obj) {
        return code;
    }

    let mut cmd = std::process::Command::new("cc");
    cmd.arg(&tmp_obj).arg(&rt_lib);
    for lib in libs {
        cmd.arg(lib);
    }
    if cfg!(target_os = "linux") {
        cmd.args(["-lpthread", "-ldl", "-lm"]);
    }
    cmd.arg("-o").arg(&bin_path);

    let status = cmd.status();
    let _ = std::fs::remove_file(&tmp_obj);

    match status {
        Ok(s) if s.success() => {
            println!("{}", bin_path.display());
            0
        }
        Ok(s) => {
            eprintln!("{file}: error: cc exited with {s}");
            1
        }
        Err(e) => {
            eprintln!("{file}: error: cannot invoke cc: {e}");
            1
        }
    }
}

// libotter_rt.a lookup order: $OTTER_RT_LIB, then the otter_fusion exe's
// directory (where `cargo build` drops the staticlib).
fn find_otter_rt() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("OTTER_RT_LIB") {
        let path = PathBuf::from(&p);
        if path.exists() {
            return Ok(path);
        }
        return Err(format!("OTTER_RT_LIB does not exist: {p}"));
    }
    let exe = std::env::current_exe().map_err(|e| format!("cannot find current exe: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| "current exe has no parent directory".to_string())?;
    let candidate = dir.join("libotter_rt.a");
    if candidate.exists() {
        return Ok(candidate);
    }
    Err(format!(
        "cannot find libotter_rt.a (looked in {}); set OTTER_RT_LIB to override",
        candidate.display()
    ))
}

fn load_dynamic_libs(paths: &[String]) -> Result<Vec<libloading::Library>, i32> {
    let mut loaded = Vec::with_capacity(paths.len());
    for p in paths {
        let path = Path::new(p);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "a" {
            eprintln!(
                "error: `run` does not accept static archives (.a): {p}; pass a .dylib/.so or use `build` for static linking"
            );
            return Err(1);
        }
        match unsafe { libloading::Library::new(path) } {
            Ok(lib) => loaded.push(lib),
            Err(e) => {
                eprintln!("error: cannot load {}: {e}", path.display());
                return Err(1);
            }
        }
    }
    Ok(loaded)
}


fn load_modules_recursively(start_file: &str, short: bool) -> Result<Vec<Module>, i32> {
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    let mut modules = Vec::new();
    let mut visited = HashSet::new();
    // Do not attempt to load core module from disk
    visited.insert("of:core".to_string());

    let mut queue = vec![PathBuf::from(start_file)];

    while let Some(path) = queue.pop() {
        let module_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("main")
            .to_string();

        if visited.contains(&module_name) {
            continue;
        }
        visited.insert(module_name.clone());

        let source = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}:1:1: error: cannot read file: {}", path.display(), e);
                return Err(1);
            }
        };

        let sm = otter_fusion::source_map::SourceMap::new(path.to_str().unwrap(), &source);

        let tokens = match Lexer::new(&source).scan_all() {
            Ok(t) => t,
            Err(e) => {
                let (line, col) = e.span();
                if short {
                    eprintln!("{}:{}:{}: error: {}", path.display(), line, col, e);
                } else {
                    print!("{}", sm.render_error(line, col, &format!("{e}")));
                }
                return Err(1);
            }
        };

        let program = match otter_fusion::parser::Parser::new(tokens).parse() {
            Ok(p) => p,
            Err(e) => {
                let (line, col) = e.span();
                if short {
                    eprintln!("{}:{}:{}: error: {}", path.display(), line, col, e);
                } else {
                    print!("{}", sm.render_error(line, col, &format!("{e}")));
                }
                return Err(1);
            }
        };

        let dir = path.parent().unwrap_or_else(|| Path::new(""));
        for item in &program.items {
            if let otter_fusion::ast::ItemKind::Import(import_decl) = &item.kind {
                let target = &import_decl.module;
                if !visited.contains(target) {
                    let next_file = dir.join(format!("{}.of", target));
                    queue.push(next_file);
                }
            }
        }

        modules.push(Module {
            name: module_name,
            program,
        });
    }

    Ok(modules)
}

fn build_hir(file: &str, short: bool) -> Result<Hir, i32> {
    let mut modules = load_modules_recursively(file, short)?;
    modules.insert(0, otter_fusion::get_core_module());

    // Fallback source map for validator errors
    let source = read_source_file(file).unwrap_or_default();
    let sm = otter_fusion::source_map::SourceMap::new(file, &source);

    Validator::new(modules).validate().map_err(|errors| {
        use otter_fusion::validator::ValidationError;
        for err in &errors {
            let (line, col) = err.span();
            
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
                eprintln!("{file}:{refined_line}:{refined_col}: error: {err}");
            } else {
                print!("{}", sm.render_error(refined_line, refined_col, &format!("{err}")));
            }
        }
        1
    })
}

// cranelift-jit 0.110 hardcodes `is_pic=true` in JITBuilder::new, which makes
// its PLT writer panic on non-x86_64. Build the ISA ourselves with is_pic off.
fn make_jit_module(libs: Vec<libloading::Library>) -> Result<JITModule, String> {
    let mut flags = cranelift_codegen::settings::builder();
    flags.set("use_colocated_libcalls", "false").map_err(|e| e.to_string())?;
    flags.set("is_pic", "false").map_err(|e| e.to_string())?;
    let isa_builder = cranelift_native::builder().map_err(|m| m.to_string())?;
    let isa = isa_builder
        .finish(cranelift_codegen::settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    let mut builder = JITBuilder::with_isa(isa, default_libcall_names());
    register_runtime_shims(&mut builder);
    if !libs.is_empty() {
        // Leak the libs: they must outlive the JIT module that resolves
        // symbols against them.
        let libs: &'static [libloading::Library] = Box::leak(libs.into_boxed_slice());
        builder.symbol_lookup_fn(Box::new(move |name: &str| {
            let cname = std::ffi::CString::new(name).ok()?;
            for lib in libs {
                unsafe {
                    if let Ok(sym) = lib.get::<*const u8>(cname.as_bytes_with_nul()) {
                        return Some(*sym);
                    }
                }
            }
            None
        }));
    }
    Ok(JITModule::new(builder))
}

// JIT resolves __of_* symbols against the otter_rt crate; AOT output links
// against libotter_rt.a producing the same symbols.
fn register_runtime_shims(builder: &mut JITBuilder) {
    builder.symbol("__of_alloc", otter_rt::__of_alloc as *const u8);
    builder.symbol("__of_vtable_lookup", otter_rt::__of_vtable_lookup as *const u8);
    builder.symbol(
        "__of_vtable_register",
        otter_rt::__of_vtable_register as *const u8,
    );
    builder.symbol("__of_vtable_clear", otter_rt::__of_vtable_clear as *const u8);
    builder.symbol("__of_print", otter_rt::__of_print as *const u8);
    builder.symbol("__of_println", otter_rt::__of_println as *const u8);
    builder.symbol("__of_str_concat", otter_rt::__of_str_concat as *const u8);
    builder.symbol("__of_str_eq", otter_rt::__of_str_eq as *const u8);
    builder.symbol("__of_i64_to_str", otter_rt::__of_i64_to_str as *const u8);
    builder.symbol("__of_u64_to_str", otter_rt::__of_u64_to_str as *const u8);
    builder.symbol("__of_f64_to_str", otter_rt::__of_f64_to_str as *const u8);
    builder.symbol("__of_bool_to_str", otter_rt::__of_bool_to_str as *const u8);
    builder.symbol("__of_char_to_str", otter_rt::__of_char_to_str as *const u8);
}

fn build_mir(file: &str) -> Result<MirProgram, i32> {
    let hir = build_hir(file, false)?;
    Lower::new(hir).lower().map_err(|e| {
        eprintln!("{file}: error: lower: {e}");
        1
    })
}
