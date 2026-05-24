pub mod ast;
pub mod codegen;
pub mod hir;
pub mod lexer;
pub mod lower;
pub mod mir;
pub mod parser;
pub mod source_map;
pub mod tokens;
pub mod validator;

/// Loads and parses the synthetic `of:core` prelude module.
/// This module is automatically injected into every compilation unit.
pub fn get_core_module() -> ast::Module {
    let source = include_str!("of_core.of");
    
    let tokens = lexer::Lexer::new(source)
        .scan_all()
        .expect("Internal error: the `of:core` synthetic module failed to lex");
        
    let program = parser::Parser::new(tokens)
        .parse()
        .expect("Internal error: the `of:core` synthetic module failed to parse");
    
    ast::Module {
        name: "of:core".to_string(),
        program,
    }
}
