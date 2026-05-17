use otter_fusion::lexer::Lexer;

fn lex(input: &str) -> Result<Vec<otter_fusion::tokens::Token>, otter_fusion::lexer::LexerError> {
    Lexer::new(input).scan_all()
}

#[test]
fn test_lexer_all() {
    insta::glob!("../examples", "**/*.of", |path| {
        let code = std::fs::read_to_string(path).unwrap();
        let tokens = lex(&code);
        insta::with_settings!({snapshot_suffix => "tokens"}, {
            insta::assert_debug_snapshot!(tokens);
        });
    });
}
