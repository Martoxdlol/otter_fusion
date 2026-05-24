use cranelift_codegen::settings::Configurable;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::default_libcall_names;
use otter_fusion::{
    ast::Module,
    codegen,
    lexer::Lexer,
    lower::Lower,
    parser::Parser,
    validator::Validator,
};

fn jit_i64(src: &str) -> i64 {
    let mut lexer = Lexer::new(src);
    let tokens = lexer.scan_all().expect("lex");
    let program = Parser::new(tokens).parse().expect("parse");
    let module = Module {
        name: "test".to_string(),
        program,
    };
    let hir = Validator::new(vec![module])
        .validate()
        .unwrap_or_else(|errs| panic!("validate: {:?}", errs));
    let mir = Lower::new(hir).lower().expect("lower");

    // cranelift-jit 0.110 hardcodes is_pic=true in JITBuilder::new which panics
    // on aarch64. Build the ISA with is_pic=false and use with_isa.
    let mut flags = cranelift_codegen::settings::builder();
    flags.set("use_colocated_libcalls", "false").unwrap();
    flags.set("is_pic", "false").unwrap();
    let isa = cranelift_native::builder()
        .expect("host isa")
        .finish(cranelift_codegen::settings::Flags::new(flags))
        .expect("isa");
    let builder = JITBuilder::with_isa(isa, default_libcall_names());
    let jit = JITModule::new(builder);
    let mut compiled = codegen::compile(&mir, jit).expect("codegen");
    compiled.module.finalize_definitions().expect("finalize");

    let entry_id = compiled.function_ids[&mir.entry];
    let ptr = compiled.module.get_finalized_function(entry_id);
    let f: extern "C" fn() -> i64 = unsafe { std::mem::transmute(ptr) };
    f()
}

#[test]
fn jit_returns_constant() {
    assert_eq!(jit_i64("function main(): i64 { 42 }"), 42);
}

#[test]
fn jit_var_and_use() {
    let src = "function main(): i64 {
        var x: i64 = 7;
        x
    }";
    assert_eq!(jit_i64(src), 7);
}

#[test]
fn jit_int_add() {
    let src = "function main(): i64 {
        var x: i64 = 5;
        var y: i64 = 10;
        x + y
    }";
    assert_eq!(jit_i64(src), 15);
}

#[test]
fn jit_int_arith_mix() {
    let src = "function main(): i64 {
        var a: i64 = 20;
        var b: i64 = 4;
        a - b * 3
    }";
    assert_eq!(jit_i64(src), 8);
}

#[test]
fn jit_signed_div_mod() {
    let src = "function main(): i64 {
        var a: i64 = 17;
        var b: i64 = 5;
        a / b * 100 + a % b
    }";
    assert_eq!(jit_i64(src), 302);
}

#[test]
fn jit_if_true_branch() {
    let src = "function main(): i64 {
        if (true) { 1 } else { 2 }
    }";
    assert_eq!(jit_i64(src), 1);
}

#[test]
fn jit_if_false_branch() {
    let src = "function main(): i64 {
        if (false) { 1 } else { 2 }
    }";
    assert_eq!(jit_i64(src), 2);
}

#[test]
fn jit_cast_widening() {
    // Use explicit `as i32` so we don't depend on the validator's
    // (currently broken) integer-literal coercion path.
    let src = "function main(): i64 {
        var x: i32 = 10 as i32;
        x as i64
    }";
    assert_eq!(jit_i64(src), 10);
}

#[test]
fn jit_cast_narrowing_roundtrip() {
    let src = "function main(): i64 {
        var x: i64 = 300;
        var y: i32 = x as i32;
        y as i64
    }";
    assert_eq!(jit_i64(src), 300);
}
