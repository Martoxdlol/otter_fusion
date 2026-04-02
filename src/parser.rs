use crate::{
    ast::{Expr, Item, Program, Statement},
    tokens::Token,
};

#[derive(Debug)]
pub struct Parser {
    // Fields for the parser
    tokens: Vec<Token>,

    current: usize, // Index of the current token
}

#[derive(Debug)]
pub enum ParserError {
    UnexpectedToken(Token),
    // Other error variants can be added here
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        // Initialize the parser
        Parser {
            // Initialize fields
            tokens,
            current: 0,
        }
    }

    pub fn parse(&mut self) -> Result<Program, ParserError> {
        // Implement parsing logic here
        Ok(Program { items: vec![] }) // Placeholder return value
    }

    pub fn parse_item(&mut self) -> Result<Item, ParserError> {
        // Implement item parsing logic here

        // parse type
        // parse interface
        // parse struct
        // parse function
        // parse extend
        todo!()
    }

    // Items
    pub fn parse_type_decl(&mut self) -> Result<Item, ParserError> {
        // Implement type declaration parsing logic here
        todo!()
    }

    pub fn parse_struct_decl(&self) -> Result<Item, ParserError> {
        // Implement struct declaration parsing logic here
        todo!()
    }

    pub fn parse_interface_decl(&self) -> Result<Item, ParserError> {
        // Implement interface declaration parsing logic here
        todo!()
    }

    pub fn parse_function_decl(&self) -> Result<Item, ParserError> {
        // Implement function declaration parsing logic here
        todo!()
    }

    pub fn parse_extend_decl(&self) -> Result<Item, ParserError> {
        // Implement extend declaration parsing logic here
        todo!()
    }

    //

    pub fn parse_statement(&self) -> Result<Statement, ParserError> {
        // Implement statement parsing logic here
        todo!()
    }

    pub fn parse_expr(&self) -> Result<Expr, ParserError> {
        // Implement expression parsing logic here
        todo!()
    }
}

/*
# Obtiene la lista de statements parseados
def parse(self) -> list[Stmt]:
    statements = []
    while not self._is_at_end():
        statements.append(self.statement())
    return statements
*/
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ast::{FieldDecl, Item, PrimitiveType, Program, StructDecl, TypeExpr},
        tokens::TokenType,
    };

    #[test]
    fn test_parse_empty_program() {
        let tokens = vec![Token {
            token_type: TokenType::EOF,
            position: 0,
            line: 1,
            column: 1,
        }];
        let mut parser = Parser::new(tokens);
        let result = parser.parse().unwrap();
        assert_eq!(result.items.len(), 0);
    }

    #[test]
    fn test_parse_simple_struct() {
        // struct Point { x: i32 }
        let tokens = vec![
            Token {
                token_type: TokenType::Struct,
                position: 0,
                line: 1,
                column: 1,
            },
            Token {
                token_type: TokenType::Identifier("Point".to_string()),
                position: 7,
                line: 1,
                column: 8,
            },
            Token {
                token_type: TokenType::LeftBrace,
                position: 13,
                line: 1,
                column: 14,
            },
            Token {
                token_type: TokenType::Identifier("x".to_string()),
                position: 17,
                line: 2,
                column: 5,
            },
            Token {
                token_type: TokenType::Colon,
                position: 18,
                line: 2,
                column: 6,
            },
            Token {
                token_type: TokenType::Identifier("i32".to_string()),
                position: 20,
                line: 2,
                column: 8,
            },
            Token {
                token_type: TokenType::RightBrace,
                position: 24,
                line: 3,
                column: 1,
            },
            Token {
                token_type: TokenType::EOF,
                position: 25,
                line: 3,
                column: 2,
            },
        ];

        let mut parser = Parser::new(tokens);
        let program = parser.parse().expect("Fallo al parsear struct básico");

        if let Item::Struct(s) = &program.items[0] {
            assert_eq!(s.name, "Point");
            assert_eq!(s.fields.len(), 1);
            assert_eq!(s.fields[0].name, "x");
        } else {
            panic!("Se esperaba un Item::Struct");
        }
    }
}
