use otter_fusion::lexer::Lexer;
use otter_fusion::parser::Parser;

fn parse(input: &str) -> Result<otter_fusion::ast::Program, String> {
    let mut lexer = Lexer::new(input);
    let tokens = lexer.scan_all().map_err(|e| e.to_string())?;

    let mut parser = Parser::new(tokens);
    let module = parser.parse().map_err(|e| e.to_string())?;
    Ok(module)
}

#[test]
fn test_parser_all() {
    insta::glob!("../examples", "**/*.of", |path| {
        let code = std::fs::read_to_string(path).unwrap();
        let ast = parse(&code);
        insta::with_settings!({snapshot_suffix => "ast"}, {
            insta::assert_debug_snapshot!(ast);
        });
    });
}
