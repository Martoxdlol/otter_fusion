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
        let generics = self.parse_generic_params()?;
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

    pub fn parse_interface_decl(&mut self) -> Result<Item, ParserError> {
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

    pub fn parse_generic_params(&mut self) -> Result<Vec<GenericParam>, ParserError> {
        if !self.expect_optional(TokenType::LT) {
            return Ok(vec![]);
        }

        let mut has_comma = false;
        let mut is_first = true;

        let mut params = Vec::new();

        loop {
            let tok = self.peek();
            match &tok.token_type {
                TokenType::Identifier(name) => {
                    if has_comma || is_first {
                        params.push(GenericParam {
                            name: name.to_string(),
                        });
                    } else {
                        return Err(ParserError::UnexpectedToken(tok.clone()));
                    }
                }
                TokenType::Comma => {
                    if has_comma || is_first {
                        return Err(ParserError::UnexpectedToken(tok.clone()));
                    }
                    has_comma = true;
                }
                TokenType::GT => {
                    self.advance();
                    break;
                }
                _ => return Err(ParserError::UnexpectedToken(tok.clone())),
            }

            self.advance();
            is_first = false;
        }

        Ok(params)
    }

    pub fn parse_implements(&mut self) -> Result<Vec<TypeExpr>, ParserError> {
        if !self.expect_optional(TokenType::Colon) {
            return Ok(vec![]);
        }

        let mut implements = Vec::new();

        loop {
            let type_expr = self.parse_type_expr()?;
            implements.push(type_expr);

            if !self.expect_optional(TokenType::Plus) {
                break;
            }
        }
        Ok(implements)
    }

    pub fn parse_type_expr(&mut self) -> Result<TypeExpr, ParserError> {
        // Implement type expression parsing logic here
        todo!()
    }

    // Struct
    pub fn parse_struct_body(
        &mut self,
    ) -> Result<(Vec<FieldDecl>, Vec<FunctionDecl>), ParserError> {
        // Implement struct body parsing logic here
        self.expect(TokenType::And)?;

        let name = self.expect_identifier()?;

        let generics = self.parse_generic_params()?;

        let implements = self.parse_implements()?;

        let

        let 
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

    pub fn expect_optional(&mut self, token_type: TokenType) -> bool {
        let token = self.peek();
        if token.token_type == token_type {
            self.advance();
            return true;
        }
        false
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
    use crate::{ast::*, tokens::TokenListBuilder};

    #[test]
    fn test_parse_empty_program() {
        let tokens = TokenListBuilder::new().eof().build();
        let mut parser = Parser::new(tokens);
        let result = parser.parse().unwrap();
        assert_eq!(result.items.len(), 0);
    }

    #[test]
    fn test_parse_simple_struct() {
        // struct Point { x: i32 }
        let tokens = TokenListBuilder::new()
            .kw_struct()
            .space()
            .identifier("Point")
            .space()
            .left_brace()
            .newline()
            .spaces(4)
            .identifier("x")
            .colon()
            .space()
            .identifier("i32")
            .newline()
            .right_brace()
            .eof()
            .build();

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
        let tokens = TokenListBuilder::new()
            .kw_type()
            .space()
            .identifier("Result")
            .space()
            .eq()
            .space()
            .identifier("i64")
            .space()
            .pipe()
            .space()
            .identifier("str")
            .semicolon()
            .eof()
            .build();

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
        let tokens = TokenListBuilder::new()
            .kw_function()
            .space()
            .identifier("add")
            .left_paren()
            .identifier("a")
            .colon()
            .space()
            .identifier("i64")
            .comma()
            .space()
            .identifier("b")
            .colon()
            .space()
            .identifier("i64")
            .right_paren()
            .colon()
            .space()
            .identifier("i64")
            .space()
            .left_brace()
            .space()
            .identifier("a")
            .space()
            .plus()
            .space()
            .identifier("b")
            .space()
            .right_brace()
            .eof()
            .build();

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

    #[test]
    fn test_parse_optional_generic_params() {
        // Simulamos: <T, U>
        let tokens = TokenListBuilder::new()
            .lt()
            .identifier("T")
            .comma()
            .identifier("U")
            .gt()
            .eof()
            .build();

        let mut parser = Parser::new(tokens);
        // Llamamos directamente al método unitario
        let generics = parser.parse_generic_params().unwrap();

        assert_eq!(generics.len(), 2);
        assert_eq!(generics[0].name, "T");
        assert_eq!(generics[1].name, "U");
    }

    #[test]
    fn test_parse_implements_clause() {
        // Simulamos: : Movable, Drawable
        let tokens = TokenListBuilder::new()
            .colon()
            .space()
            .identifier("Movable")
            .comma()
            .space()
            .identifier("Drawable")
            .eof()
            .build();

        let mut parser = Parser::new(tokens);
        let implements = parser.parse_implements().unwrap();

        assert_eq!(implements.len(), 2);
        assert_eq!(
            implements[0],
            TypeExpr::Named("Movable".to_string(), vec![])
        );
        assert_eq!(
            implements[1],
            TypeExpr::Named("Drawable".to_string(), vec![])
        );
    }

    #[test]
    fn test_parse_type_decl_unit() {
        // Simulamos: type Age = i32;
        let tokens = TokenListBuilder::new()
            .kw_type()
            .space()
            .identifier("Age")
            .space()
            .eq()
            .space()
            .identifier("i32")
            .semicolon()
            .eof()
            .build();

        let mut parser = Parser::new(tokens);
        // Consumimos directamente la declaración de tipo
        let result = parser.parse_type_decl().unwrap();

        let expected = Item::TypeAlias(TypeAliasDecl {
            name: "Age".to_string(),
            generics: vec![],
            ty: TypeExpr::Named("i32".to_string(), vec![]),
        });

        assert_eq!(result, expected);
    }

    #[test]
    fn test_parse_interface_decl_unit() {
        // Simulamos: interface Named { name: str }
        let tokens = TokenListBuilder::new()
            .kw_interface()
            .space()
            .identifier("Named")
            .space()
            .left_brace()
            .space()
            .identifier("name")
            .colon()
            .space()
            .identifier("str")
            .space()
            .right_brace()
            .eof()
            .build();

        let mut parser = Parser::new(tokens);
        let result = parser.parse_interface_decl().unwrap();

        let expected = Item::Interface(InterfaceDecl {
            name: "Named".to_string(),
            generics: vec![],
            fields: vec![FieldDecl {
                name: "name".to_string(),
                ty: TypeExpr::Named("str".to_string(), vec![]),
            }],
            methods: vec![],
        });

        assert_eq!(result, expected);
    }

    #[test]
    fn test_parse_extend_decl_unit() {
        // Simulamos: extend Vehicle: Movable {}
        let tokens = TokenListBuilder::new()
            .kw_extend()
            .space()
            .identifier("Vehicle")
            .colon()
            .space()
            .identifier("Movable")
            .space()
            .left_brace()
            .right_brace()
            .eof()
            .build();

        let mut parser = Parser::new(tokens);
        let result = parser.parse_extend_decl().unwrap();

        let expected = Item::Extend(ExtendDecl {
            target: TypeExpr::Named("Vehicle".to_string(), vec![]),
            generic_params: vec![],
            implements: vec![TypeExpr::Named("Movable".to_string(), vec![])],
            methods: vec![],
        });

        assert_eq!(result, expected);
    }
}
