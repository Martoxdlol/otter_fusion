use crate::{
    ast::{
        Expr, FieldDecl, FunctionDecl, GenericParam, Item, Program, Statement, StructDecl, TypeExpr,
    },
    tokens::{Token, TokenType},
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
        let mut program = Program { items: vec![] };

        while !self.is_at_end() {
            let item = self.parse_item()?;
            program.items.push(item);
        }

        Ok(program)
    }

    pub fn parse_item(&mut self) -> Result<Item, ParserError> {
        // Implement item parsing logic here
        let token = self.peek();
        match &token.token_type {
            TokenType::Type => self.parse_type_decl(),
            TokenType::Struct => Ok(Item::Struct(self.parse_struct_decl()?)),
            TokenType::Interface => self.parse_interface_decl(),
            TokenType::Function => self.parse_function_decl(),
            TokenType::Extend => self.parse_extend_decl(),
            _ => Err(ParserError::UnexpectedToken(token.clone())),
        }
    }

    // Items
    pub fn parse_type_decl(&mut self) -> Result<Item, ParserError> {
        // Implement type declaration parsing logic here
        todo!()
    }

    pub fn parse_struct_decl(&mut self) -> Result<StructDecl, ParserError> {
        self.expect(TokenType::Struct)?;
        let generics = self.parse_optional_generic_params()?;
        let name = self.expect_identifier()?;
        let implements = self.parse_implements()?;
        let (fields, methods) = self.parse_struct_body()?;
        Ok(StructDecl {
            name,
            fields,
            generics,
            implements,
            methods,
        })
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

    // Types

    pub fn parse_optional_generic_params(&mut self) -> Result<Vec<GenericParam>, ParserError> {
        // Implement optional generic parameters parsing logic here
        Ok(vec![])
    }

    pub fn parse_implements(&mut self) -> Result<Vec<TypeExpr>, ParserError> {
        // Implement implements parsing logic here
        Ok(vec![])
    }

    // Struct
    pub fn parse_struct_body(
        &mut self,
    ) -> Result<(Vec<FieldDecl>, Vec<FunctionDecl>), ParserError> {
        // Implement struct body parsing logic here
        Ok((vec![], vec![]))
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

    // Util

    fn is_at_end(&self) -> bool {
        self.peek().token_type == TokenType::EOF
    }

    pub fn peek(&self) -> &Token {
        self.tokens.get(self.current).unwrap_or(&Token {
            token_type: TokenType::EOF,
            position: 0,
            line: 0,
            column: 0,
        })
    }

    pub fn advance(&mut self) {
        if self.current < self.tokens.len() {
            self.current += 1;
        }
    }

    pub fn expect(&mut self, token_type: TokenType) -> Result<(), ParserError> {
        let token = self.peek();
        if token.token_type == token_type {
            self.advance();
            return Ok(());
        }

        Err(ParserError::UnexpectedToken(token.clone()))
    }

    pub fn expect_identifier(&mut self) -> Result<String, ParserError> {
        let token = self.peek();
        if let TokenType::Identifier(name) = &token.token_type {
            let name_cloned = name.clone();
            self.advance();
            return Ok(name_cloned);
        }

        Err(ParserError::UnexpectedToken(token.clone()))
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
    use crate::{ast::*, tokens::TokenType};

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

    #[test]
    fn test_parse_type_alias_union() {
        // type Result = i64 | str;
        let tokens = vec![
            Token {
                token_type: TokenType::Type,
                position: 0,
                line: 1,
                column: 1,
            },
            Token {
                token_type: TokenType::Identifier(String::from("Result")),
                position: 5,
                line: 1,
                column: 6,
            },
            Token {
                token_type: TokenType::Eq,
                position: 12,
                line: 1,
                column: 13,
            },
            Token {
                token_type: TokenType::Identifier(String::from("i64")),
                position: 14,
                line: 1,
                column: 15,
            },
            Token {
                token_type: TokenType::Pipe,
                position: 18,
                line: 1,
                column: 19,
            },
            Token {
                token_type: TokenType::Identifier(String::from("str")),
                position: 20,
                line: 1,
                column: 21,
            },
            Token {
                token_type: TokenType::Semicolon,
                position: 23,
                line: 1,
                column: 24,
            },
            Token {
                token_type: TokenType::EOF,
                position: 24,
                line: 1,
                column: 25,
            },
        ];

        let mut parser = Parser::new(tokens);
        let program = parser.parse().unwrap();

        let expected_alias = TypeAliasDecl {
            name: String::from("Result"),
            generics: vec![],
            ty: TypeExpr::Union(vec![
                TypeExpr::Named(String::from("i64"), vec![]),
                TypeExpr::Named(String::from("str"), vec![]),
            ]),
        };

        assert_eq!(program.items.len(), 1);
        assert_eq!(program.items[0], Item::TypeAlias(expected_alias));
    }

    #[test]
    fn test_parse_function_implicit_return() {
        // function add(a: i64, b: i64): i64 { a + b }
        let tokens = vec![
            Token {
                token_type: TokenType::Function,
                position: 0,
                line: 1,
                column: 1,
            },
            Token {
                token_type: TokenType::Identifier(String::from("add")),
                position: 9,
                line: 1,
                column: 10,
            },
            Token {
                token_type: TokenType::LeftParen,
                position: 12,
                line: 1,
                column: 13,
            },
            Token {
                token_type: TokenType::Identifier(String::from("a")),
                position: 13,
                line: 1,
                column: 14,
            },
            Token {
                token_type: TokenType::Colon,
                position: 14,
                line: 1,
                column: 15,
            },
            Token {
                token_type: TokenType::Identifier(String::from("i64")),
                position: 16,
                line: 1,
                column: 17,
            },
            Token {
                token_type: TokenType::Comma,
                position: 19,
                line: 1,
                column: 20,
            },
            Token {
                token_type: TokenType::Identifier(String::from("b")),
                position: 21,
                line: 1,
                column: 22,
            },
            Token {
                token_type: TokenType::Colon,
                position: 22,
                line: 1,
                column: 23,
            },
            Token {
                token_type: TokenType::Identifier(String::from("i64")),
                position: 24,
                line: 1,
                column: 25,
            },
            Token {
                token_type: TokenType::RightParen,
                position: 27,
                line: 1,
                column: 28,
            },
            Token {
                token_type: TokenType::Colon,
                position: 28,
                line: 1,
                column: 29,
            },
            Token {
                token_type: TokenType::Identifier(String::from("i64")),
                position: 30,
                line: 1,
                column: 31,
            },
            Token {
                token_type: TokenType::LeftBrace,
                position: 34,
                line: 1,
                column: 35,
            },
            Token {
                token_type: TokenType::Identifier(String::from("a")),
                position: 36,
                line: 1,
                column: 37,
            },
            Token {
                token_type: TokenType::Plus,
                position: 38,
                line: 1,
                column: 39,
            },
            Token {
                token_type: TokenType::Identifier(String::from("b")),
                position: 40,
                line: 1,
                column: 41,
            },
            Token {
                token_type: TokenType::RightBrace,
                position: 42,
                line: 1,
                column: 43,
            },
            Token {
                token_type: TokenType::EOF,
                position: 43,
                line: 1,
                column: 44,
            },
        ];

        let mut parser = Parser::new(tokens);
        let program = parser.parse().unwrap();

        let expected_func = FunctionDecl {
            name: String::from("add"),
            has_self_param: false,
            generics: vec![],
            return_type: TypeExpr::Named(String::from("i64"), vec![]),
            params: vec![
                ParamDecl {
                    name: String::from("a"),
                    ty: TypeExpr::Named(String::from("i64"), vec![]),
                },
                ParamDecl {
                    name: String::from("b"),
                    ty: TypeExpr::Named(String::from("i64"), vec![]),
                },
            ],
            body: Some(Block {
                statements: vec![], // Vacío porque no hay punto y coma
                returns: Some(Expr::BinaryOp(
                    Box::new(Expr::Variable(String::from("a"))),
                    BinaryOperator::Add,
                    Box::new(Expr::Variable(String::from("b"))),
                )),
            }),
        };

        assert_eq!(program.items.len(), 1);
        assert_eq!(program.items[0], Item::Function(expected_func));
    }
}
