use otter_fusion::lexer::Lexer;
use otter_fusion::parser::Parser;
use otter_fusion::validator::Validator;

/// Result of running the full compilation pipeline on a single file.
/// Captures which phase failed so snapshots are transparent.
#[derive(Debug)]
enum CompileResult {
    LexerError(String),
    ParserError(String),
    ValidationErrors(Vec<String>),
    Ok,
}

fn compile(input: &str) -> CompileResult {
    let mut lexer = Lexer::new(input);
    let tokens = match lexer.scan_all() {
        Ok(t) => t,
        Err(e) => return CompileResult::LexerError(format!("{}", e)),
    };

    let mut parser = Parser::new(tokens);
    let program = match parser.parse() {
        Ok(p) => p,
        Err(e) => return CompileResult::ParserError(format!("{}", e)),
    };

    let module = otter_fusion::ast::Module {
        name: "test_module".to_string(),
        program,
    };

    match Validator::new(vec![module]).validate() {
        Ok(_) => CompileResult::Ok,
        Err(mut errs) => {
            errs.sort_by_key(|e| format!("{:?}", e));
            CompileResult::ValidationErrors(
                errs.iter().map(|e| format!("{}", e)).collect(),
            )
        }
    }
}

#[test]
fn test_validator_all() {
    insta::glob!("../examples", "**/*.of", |path| {
        let code = std::fs::read_to_string(path).unwrap();
        let result = compile(&code);
        insta::with_settings!({snapshot_suffix => "validator"}, {
            insta::assert_debug_snapshot!(result);
        });
    });
}
