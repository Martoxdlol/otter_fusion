use otter_fusion::{
    ast::Module,
    hir::PrimitiveType,
    lexer::Lexer,
    lower::Lower,
    mir::{
        Abi, AssignValue, BinOp, Callee, MirConst, MirProgram, MirType, MirTypeDef, Operand, Stmt,
        Terminator,
    },
    parser::Parser,
    validator::Validator,
};

fn lower_source(src: &str) -> MirProgram {
    let mut lexer = Lexer::new(src);
    let tokens = lexer.scan_all().expect("lex");
    let mut parser = Parser::new(tokens);
    let program = parser.parse().expect("parse");
    let module = Module {
        name: "test".to_string(),
        program,
    };
    // Real compilation always prepends of:core (see src/main.rs); mirror
    // that here so tests that `import` from it can validate.
    let modules = vec![otter_fusion::get_core_module(), module];
    let hir = Validator::new(modules)
        .validate()
        .unwrap_or_else(|errs| panic!("validate: {:?}", errs));
    Lower::new(hir).lower().expect("lower")
}

fn entry_fn(mir: &MirProgram) -> &otter_fusion::mir::MirFunction {
    mir.functions.get(&mir.entry).expect("entry function")
}

#[test]
fn lowers_empty_main() {
    let mir = lower_source("function main(): i64 { 0 }");
    let f = entry_fn(&mir);
    assert_eq!(f.name, "test::main");
    assert!(matches!(f.return_type, MirType::Primitive(PrimitiveType::Int64)));
    let blk = &f.blocks[&f.entry];
    match &blk.terminator {
        Terminator::Return(Some(Operand::Const(MirConst::Int(0, PrimitiveType::Int64)))) => {}
        other => panic!("unexpected terminator: {:?}", other),
    }
}

#[test]
fn lowers_var_decl_and_use() {
    let mir = lower_source(
        "function main(): i64 {
            var x: i64 = 42;
            x
        }",
    );
    let f = entry_fn(&mir);
    let blk = &f.blocks[&f.entry];
    // Should have one Assign for the VarDecl init.
    assert_eq!(blk.stmts.len(), 1);
    match &blk.stmts[0] {
        Stmt::Assign(_, AssignValue::Use(Operand::Const(MirConst::Int(42, _)))) => {}
        other => panic!("unexpected stmt: {:?}", other),
    }
    assert!(matches!(blk.terminator, Terminator::Return(Some(_))));
}

#[test]
fn lowers_binary_arithmetic() {
    let mir = lower_source(
        "function main(): i64 {
            var x: i64 = 5;
            var y: i64 = 10;
            x + y
        }",
    );
    let f = entry_fn(&mir);
    let blk = &f.blocks[&f.entry];
    // Three stmts: x = 5, y = 10, tmp = x + y. Return(tmp).
    let bin_stmt = blk
        .stmts
        .iter()
        .find(|s| matches!(s, Stmt::Assign(_, AssignValue::Bin(BinOp::Add, _, _))))
        .expect("expected Add stmt");
    if let Stmt::Assign(_, AssignValue::Bin(BinOp::Add, _, _)) = bin_stmt {
        // good
    }
    assert!(matches!(blk.terminator, Terminator::Return(Some(_))));
}

#[test]
fn lowers_unary_not() {
    let mir = lower_source(
        "function main(): i64 {
            var b: bool = !false;
            if (b) { 1 } else { 0 }
        }",
    );
    let f = entry_fn(&mir);
    let has_un = f.blocks.values().any(|bb| {
        bb.stmts
            .iter()
            .any(|s| matches!(s, Stmt::Assign(_, AssignValue::Un(_, _))))
    });
    assert!(has_un, "expected a unary op in the lowered MIR");
}

#[test]
fn lowers_numeric_cast() {
    let mir = lower_source(
        "function main(): i64 {
            var x: i32 = 10;
            x as i64
        }",
    );
    let f = entry_fn(&mir);
    let blk = &f.blocks[&f.entry];
    let has_cast = blk
        .stmts
        .iter()
        .any(|s| matches!(s, Stmt::Assign(_, AssignValue::Cast(_, MirType::Primitive(PrimitiveType::Int64)))));
    assert!(has_cast, "expected a Cast to Int64");
}

#[test]
fn lowers_if_expression_creates_cfg() {
    let mir = lower_source(
        "function main(): i64 {
            if (true) { 1 } else { 2 }
        }",
    );
    let f = entry_fn(&mir);
    // entry CondBr, two arms, join block. At least 4 blocks total.
    assert!(
        f.blocks.len() >= 4,
        "expected >=4 blocks for if, got {}",
        f.blocks.len()
    );
    let entry_block = &f.blocks[&f.entry];
    assert!(matches!(entry_block.terminator, Terminator::CondBr(_, _, _)));
}

#[test]
fn lowers_block_expression() {
    let mir = lower_source(
        "function main(): i64 {
            var v: i64 = {
                var a: i64 = 1;
                var b: i64 = 2;
                a + b
            };
            v
        }",
    );
    let f = entry_fn(&mir);
    let blk = &f.blocks[&f.entry];
    let adds = blk
        .stmts
        .iter()
        .filter(|s| matches!(s, Stmt::Assign(_, AssignValue::Bin(BinOp::Add, _, _))))
        .count();
    assert_eq!(adds, 1, "expected the inner a + b to appear inline");
}

#[test]
fn lowers_struct_init_and_member() {
    let mir = lower_source(
        "struct Point {
            x: i64
            y: i64
        }
        function main(): i64 {
            var p: Point = Point { x: 3, y: 4 };
            p.x + p.y
        }",
    );
    let f = entry_fn(&mir);
    let blk = &f.blocks[&f.entry];

    let has_alloc = blk.stmts.iter().any(|s| {
        matches!(
            s,
            Stmt::Assign(_, AssignValue::AllocStruct(_, ops)) if ops.len() == 2
        )
    });
    assert!(has_alloc, "expected AllocStruct with two ops");

    let field_reads = blk
        .stmts
        .iter()
        .filter(|s| matches!(s, Stmt::Assign(_, AssignValue::Field(_, _))))
        .count();
    assert_eq!(field_reads, 2, "expected two field reads (p.x and p.y)");

    // Struct lives in mir.types as a Struct with 2 fields.
    let struct_def = mir
        .types
        .values()
        .find(|t| matches!(t, MirTypeDef::Struct { fields, .. } if fields.len() == 2))
        .expect("Point struct in mir.types");
    if let MirTypeDef::Struct { fields, layout, .. } = struct_def {
        assert_eq!(fields[0].name, "x");
        assert_eq!(fields[1].name, "y");
        assert_eq!(fields[0].offset, 0);
        assert_eq!(fields[1].offset, 8);
        assert_eq!(layout.size, 16);
        assert_eq!(layout.align, 8);
    }
}

#[test]
fn lowers_inherent_method_call() {
    let mir = lower_source(
        "struct Box {
            n: i64

            function get(self): i64 { self.n }
        }
        function main(): i64 {
            var b: Box = Box { n: 42 };
            b.get()
        }",
    );
    // Two user-defined functions: main and Box::get.
    let names: Vec<&str> = mir.functions.values().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"test::main"));
    assert!(
        names.iter().any(|n| n.contains("get")),
        "expected a get function, got {:?}",
        names
    );

    let main = entry_fn(&mir);
    let has_call = main.blocks.values().any(|bb| {
        bb.stmts.iter().any(|s| {
            matches!(
                s,
                Stmt::Assign(_, AssignValue::Call(Callee::Static(_), _))
            )
        })
    });
    assert!(has_call, "expected a static method call in main");
}

#[test]
fn lowers_while_loop() {
    let mir = lower_source(
        "function main(): i64 {
            while (false) { }
            0
        }",
    );
    let f = entry_fn(&mir);
    // Entry -> head_bb (Goto), head -> CondBr, body Goto head, exit Return.
    let cond_branches = f
        .blocks
        .values()
        .filter(|bb| matches!(bb.terminator, Terminator::CondBr(_, _, _)))
        .count();
    assert!(cond_branches >= 1, "expected a CondBr from while head");
    let gotos = f
        .blocks
        .values()
        .filter(|bb| matches!(bb.terminator, Terminator::Goto(_)))
        .count();
    assert!(gotos >= 1, "expected at least one Goto");
}

#[test]
fn lowers_call_to_void_extern_via_print() {
    let mir = lower_source(
        r#"import { print } from "of:core";
        function main(): i64 {
            print("hello");
            0
        }"#,
    );

    let names: Vec<&str> = mir.functions.values().map(|f| f.name.as_str()).collect();
    let stub = mir
        .functions
        .values()
        .find(|f| f.name == "__of_print")
        .unwrap_or_else(|| panic!("__of_print stub not found; have: {:?}", names));
    assert!(matches!(stub.abi, Abi::Extern));
    assert_eq!(stub.blocks.len(), 0, "extern stub has no body");
    assert_eq!(stub.params.len(), 1);

    // The intermediate `print` wrapper has a body (calls __of_print).
    let wrapper = mir
        .functions
        .values()
        .find(|f| f.name.ends_with("::print"))
        .expect("print wrapper");
    assert!(matches!(wrapper.abi, Abi::Otter));
    assert!(!wrapper.blocks.is_empty());
}

#[test]
fn struct_layout_respects_declared_field_order() {
    let mir = lower_source(
        "struct Mixed {
            a: i8
            b: i64
            c: i8
        }
        function main(): i64 {
            var m: Mixed = Mixed { a: 1, b: 2, c: 3 };
            0
        }",
    );
    let mixed = mir
        .types
        .values()
        .find(|t| matches!(t, MirTypeDef::Struct { fields, .. } if fields.len() == 3))
        .expect("Mixed struct");
    if let MirTypeDef::Struct { fields, layout, .. } = mixed {
        // a@0 (i8), pad to 8, b@8, c@16, total padded to align 8 = 24.
        assert_eq!(fields[0].name, "a");
        assert_eq!(fields[0].offset, 0);
        assert_eq!(fields[1].name, "b");
        assert_eq!(fields[1].offset, 8);
        assert_eq!(fields[2].name, "c");
        assert_eq!(fields[2].offset, 16);
        assert_eq!(layout.size, 24);
        assert_eq!(layout.align, 8);
    }
}
