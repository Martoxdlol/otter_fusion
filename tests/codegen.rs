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

fn register_rt_shims(builder: &mut JITBuilder) {
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

fn build_jit() -> JITModule {
    // cranelift-jit 0.110 hardcodes is_pic=true in JITBuilder::new which panics
    // on aarch64. Build the ISA with is_pic=false and use with_isa.
    let mut flags = cranelift_codegen::settings::builder();
    flags.set("use_colocated_libcalls", "false").unwrap();
    flags.set("is_pic", "false").unwrap();
    let isa = cranelift_native::builder()
        .expect("host isa")
        .finish(cranelift_codegen::settings::Flags::new(flags))
        .expect("isa");
    let mut builder = JITBuilder::with_isa(isa, default_libcall_names());
    register_rt_shims(&mut builder);
    JITModule::new(builder)
}

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

    let jit = build_jit();
    let mut compiled = codegen::compile(&mir, jit).expect("codegen");
    let init_fid = codegen::emit_vtable_init(&mut compiled, &mir).expect("vtable init");
    compiled.module.finalize_definitions().expect("finalize");

    let init_ptr = compiled.module.get_finalized_function(init_fid);
    let init: extern "C" fn() = unsafe { std::mem::transmute(init_ptr) };
    init();

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

// ---- Virtual dispatch via interface vtables ----

#[test]
fn jit_virtual_single_struct() {
    let src = "
        interface Shape {
          function area(self): i64
        }
        struct Circle: Shape {
          r: i64,
          function area(self): i64 { self.r * self.r * 3 }
        }
        function area_of(s: Shape): i64 { s.area() }
        function main(): i64 {
          var c = Circle { r: 5 };
          area_of(c)
        }
    ";
    assert_eq!(jit_i64(src), 75);
}

#[test]
fn jit_virtual_two_structs_same_interface() {
    // Same interface parameter receives two different struct types; each
    // call must resolve to that struct's own method.
    let src = "
        interface Shape {
          function area(self): i64
        }
        struct Circle: Shape {
          r: i64,
          function area(self): i64 { self.r * self.r * 3 }
        }
        struct Square: Shape {
          s: i64,
          function area(self): i64 { self.s * self.s }
        }
        function area_of(s: Shape): i64 { s.area() }
        function main(): i64 {
          var c = Circle { r: 5 };
          var sq = Square { s: 4 };
          area_of(c) + area_of(sq)
        }
    ";
    assert_eq!(jit_i64(src), 91); // 75 + 16
}

#[test]
fn jit_virtual_inherited_interface() {
    // Calling a parent-interface method on a child-interface receiver must
    // also work — the (struct, parent) vtable is built by lowering.
    let src = "
        interface A {
          function tag(self): i64
        }
        interface B: A {
          function extra(self): i64
        }
        struct Box: B {
          v: i64,
          function tag(self): i64 { self.v }
          function extra(self): i64 { self.v * 10 }
        }
        function as_a(x: A): i64 { x.tag() }
        function as_b(x: B): i64 { x.extra() + x.tag() }
        function main(): i64 {
          var box = Box { v: 7 };
          as_a(box) + as_b(box)
        }
    ";
    // as_a(box) = 7; as_b(box) = 70 + 7 = 77; sum = 84.
    assert_eq!(jit_i64(src), 84);
}

#[test]
fn jit_virtual_returns_via_field() {
    // Dispatches against a method that reads multiple struct fields, to
    // make sure the receiver pointer is passed and the GC header offset
    // doesn't clobber field 0.
    let src = "
        interface Sum {
          function total(self): i64
        }
        struct Pair: Sum {
          a: i64,
          b: i64,
          function total(self): i64 { self.a + self.b }
        }
        function call(s: Sum): i64 { s.total() }
        function main(): i64 {
          var p = Pair { a: 100, b: 23 };
          call(p)
        }
    ";
    assert_eq!(jit_i64(src), 123);
}
