use crate::{
    ast::{
        Block, Expr, ExtendDecl, FieldDecl, FunctionDecl, GenericParam, InterfaceDecl, Item,
        ParamDecl, PrimitiveType, Program, Statement, StructDecl, TypeAliasDecl, TypeExpr,
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
            TokenType::Function => Ok(Item::Function(self.parse_function_decl()?)),
            TokenType::Extend => self.parse_extend_decl(),
            _ => Err(ParserError::UnexpectedToken(token.clone())),
        }
    }

    // Items
    pub fn parse_type_decl(&mut self) -> Result<Item, ParserError> {
        self.expect(TokenType::Type)?;
        let name = self.expect_identifier()?;
        let generics = self.parse_generic_params()?;
        self.expect(TokenType::Eq)?;
        let ty = self.parse_type_expr()?;
        self.expect(TokenType::Semicolon)?;
        Ok(Item::TypeAlias(TypeAliasDecl { name, generics, ty }))
    }

    pub fn parse_struct_decl(&mut self) -> Result<StructDecl, ParserError> {
        self.expect(TokenType::Struct)?;
        let name = self.expect_identifier()?;
        let generics = self.parse_generic_params()?;
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
        self.expect(TokenType::Interface)?;
        let name = self.expect_identifier()?;
        let generics = self.parse_generic_params()?;
        let implements = self.parse_implements()?;
        let (fields, methods) = self.parse_struct_body()?;
        Ok(Item::Interface(InterfaceDecl {
            name,
            generics,
            implements,
            fields,
            methods,
        }))
    }

    pub fn parse_function_decl(&mut self) -> Result<FunctionDecl, ParserError> {
        self.expect(TokenType::Function)?;
        let name = self.expect_identifier()?;
        let generics = self.parse_generic_params()?;
        let mut params = self.parse_function_args()?;
        let mut return_type = None;
        if self.expect_optional(TokenType::Colon) {
            return_type = Some(self.parse_type_expr()?);
        }

        let has_self_param = params.first().map_or(false, |p| p.name == "self");

        if has_self_param {
            params.remove(0);
        }

        let body = if self.peek().token_type == TokenType::Semicolon {
            self.advance();
            None
        } else {
            Some(self.parse_block()?)
        };

        Ok(FunctionDecl {
            name,
            has_self_param,
            generics,
            return_type,
            params,
            body,
        })
    }

    pub fn parse_extend_decl(&mut self) -> Result<Item, ParserError> {
        self.expect(TokenType::Extend)?;
        let generics = self.parse_generic_params()?;
        let name = self.expect_identifier()?;
        let type_args = self.parse_type_args()?;
        let implements = self.parse_implements()?;

        let methods = self.parse_extend_body()?;

        Ok(Item::Extend(ExtendDecl {
            target: TypeExpr::Named(name, type_args),
            generic_params: generics,
            implements,
            methods,
        }))
    }

    // Types

    pub fn parse_generic_params(&mut self) -> Result<Vec<GenericParam>, ParserError> {
        if !self.expect_optional(TokenType::LT) {
            return Ok(vec![]);
        }

        let mut params = Vec::new();

        if self.peek().token_type != TokenType::GT {
            params.push(self.parse_generic_param()?);
            while self.expect_optional(TokenType::Comma) {
                params.push(self.parse_generic_param()?);
            }
        }

        self.expect(TokenType::GT)?;
        Ok(params)
    }

    fn parse_generic_param(&mut self) -> Result<GenericParam, ParserError> {
        let name = self.expect_identifier()?;
        let bounds = if self.expect_optional(TokenType::Colon) {
            let mut bounds = vec![self.parse_type_atom()?];
            while self.expect_optional(TokenType::Plus) {
                bounds.push(self.parse_type_atom()?);
            }
            bounds
        } else {
            vec![]
        };
        Ok(GenericParam { name, bounds })
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
        let ty = self.parse_type_atom()?;

        // Union type: type1 | type2 | ...
        if self.peek().token_type == TokenType::Pipe {
            let mut variants = vec![ty];
            while self.expect_optional(TokenType::Pipe) {
                variants.push(self.parse_type_atom()?);
            }
            Ok(TypeExpr::Union(variants))
        } else {
            Ok(ty)
        }
    }

    fn parse_type_atom(&mut self) -> Result<TypeExpr, ParserError> {
        let token = self.peek().clone();
        match &token.token_type {
            TokenType::Identifier(name) => {
                if let Some(primitive) = self.parse_optional_primitive(name) {
                    self.advance();
                    Ok(TypeExpr::Primitive(primitive))
                } else {
                    let name = name.clone();
                    self.advance();
                    let args = self.parse_type_args()?;
                    Ok(TypeExpr::Named(name, args))
                }
            }
            TokenType::Null => {
                self.advance();
                Ok(TypeExpr::Primitive(PrimitiveType::Null))
            }
            // Function type: (param_types) -> return_type
            TokenType::LeftParen => {
                self.advance();
                let mut param_types = Vec::new();
                if self.peek().token_type != TokenType::RightParen {
                    param_types.push(self.parse_type_expr()?);
                    while self.expect_optional(TokenType::Comma) {
                        param_types.push(self.parse_type_expr()?);
                    }
                }
                self.expect(TokenType::RightParen)?;
                self.expect(TokenType::Minus)?;
                self.expect(TokenType::GT)?;
                let return_type = self.parse_type_expr()?;
                Ok(TypeExpr::Function(param_types, Box::new(return_type)))
            }
            _ => Err(ParserError::UnexpectedToken(token)),
        }
    }

    pub fn parse_type_args(&mut self) -> Result<Vec<TypeExpr>, ParserError> {
        if !self.expect_optional(TokenType::LT) {
            return Ok(vec![]);
        }

        let mut args = Vec::new();

        if self.peek().token_type != TokenType::GT {
            args.push(self.parse_type_expr()?);
            while self.expect_optional(TokenType::Comma) {
                args.push(self.parse_type_expr()?);
            }
        }

        self.expect(TokenType::GT)?;
        Ok(args)
    }

    pub fn parse_field(&mut self) -> Result<FieldDecl, ParserError> {
        let name = self.expect_identifier()?;
        self.expect(TokenType::Colon)?;
        let ty = self.parse_type_expr()?;
        Ok(FieldDecl { name, ty })
    }

    // Struct
    pub fn parse_struct_body(
        &mut self,
    ) -> Result<(Vec<FieldDecl>, Vec<FunctionDecl>), ParserError> {
        self.expect(TokenType::LeftBrace)?;

        // Implement struct body parsing logic here
        let mut fields: Vec<FieldDecl> = Vec::new();
        let mut methods: Vec<FunctionDecl> = Vec::new();

        loop {
            let tok = self.peek();
            match &tok.token_type {
                TokenType::Identifier(_) => fields.push(self.parse_field()?),
                TokenType::Function => methods.push(self.parse_function_decl()?),
                TokenType::RightBrace => {
                    self.advance();
                    break;
                }
                _ => return Err(ParserError::UnexpectedToken(tok.clone())),
            }
        }

        Ok((fields, methods))
    }

    pub fn parse_extend_body(&mut self) -> Result<Vec<FunctionDecl>, ParserError> {
        self.expect(TokenType::LeftBrace)?;

        // Implement struct body parsing logic here
        let mut methods: Vec<FunctionDecl> = Vec::new();

        loop {
            let tok = self.peek();
            match &tok.token_type {
                TokenType::Function => methods.push(self.parse_function_decl()?),
                TokenType::RightBrace => {
                    self.advance();
                    break;
                }
                _ => return Err(ParserError::UnexpectedToken(tok.clone())),
            }
        }

        Ok(methods)
    }

    // Functions

    pub fn parse_function_args(&mut self) -> Result<Vec<ParamDecl>, ParserError> {
        self.expect(TokenType::LeftParen)?;
        let mut params = Vec::new();

        if self.peek().token_type != TokenType::RightParen {
            params.push(self.parse_param_decl()?);
            while self.expect_optional(TokenType::Comma) {
                params.push(self.parse_param_decl()?);
            }
        }

        self.expect(TokenType::RightParen)?;
        Ok(params)
    }

    pub fn parse_param_decl(&mut self) -> Result<ParamDecl, ParserError> {
        let name = self.expect_identifier()?;
        self.expect(TokenType::Colon)?;
        let ty = self.parse_type_expr()?;
        Ok(ParamDecl { name, ty })
    }

    pub fn parse_block(&mut self) -> Result<Block, ParserError> {
        if self.peek().token_type == TokenType::Semicolon {
            self.advance();
            return Ok(Block {
                statements: vec![],
                returns: None,
            });
        }

        self.expect(TokenType::LeftBrace)?;
        // Implement block parsing logic here
        let mut statements = Vec::new();
        let mut final_expression: Option<Expr> = None;
        loop {
            let tok = self.peek();

            if tok.token_type == TokenType::RightBrace {
                self.advance();
                break;
            }

            statements.push(self.parse_statement()?);

            if self.peek().token_type != TokenType::RightBrace {
                final_expression = Some(self.parse_expr()?);
                self.expect(TokenType::RightBrace)?;
                break;
            } else {
                self.expect(TokenType::Semicolon)?;
            }
        }

        Ok(Block {
            statements,
            returns: final_expression,
        })
    }

    //

    pub fn parse_statement(&mut self) -> Result<Statement, ParserError> {
        // Implement statement parsing logic here
        // var x = 1
        match &self.peek().token_type {
            TokenType::Var => {
                self.expect(TokenType::Var)?;
                let name = self.expect_identifier()?;
                let ty: Option<TypeExpr> = if self.expect_optional(TokenType::Colon) {
                    Some(self.parse_type_expr()?)
                } else {
                    None
                };

                let value: Option<Expr> = if self.expect_optional(TokenType::Eq) {
                    Some(self.parse_expr()?)
                } else {
                    None
                };

                Ok(Statement::VarDecl(name, ty, value))
            }
            TokenType::Return => {
                self.expect(TokenType::Return)?;

                if self.peek().token_type == TokenType::Semicolon
                    || self.peek().token_type == TokenType::RightBrace
                {
                    return Ok(Statement::Return(None));
                }

                let expr = self.parse_expr()?;
                Ok(Statement::Return(Some(expr)))
            }
            TokenType::While => {
                self.expect(TokenType::While)?;
                let condition = self.parse_expr()?;
                let body = self.parse_block()?;
                Ok(Statement::While(condition, body))
            }

            TokenType::For => {
                self.expect(TokenType::For)?;
                let name = self.expect_identifier()?;
                self.expect(TokenType::In)?;
                let iterable = self.parse_expr()?;
                let body = self.parse_block()?;
                Ok(Statement::For(name, iterable, body))
            }
            TokenType::Break => Ok(Statement::Break),
            TokenType::Continue => Ok(Statement::Continue),
            _ => Ok(Statement::Expr(self.parse_expr()?)),
        }
    }

    pub fn parse_expr(&self) -> Result<Expr, ParserError> {
        // Implement expression parsing logic here
        match self.peek().token_type {
            TokenType::LeftBrace => todo!(),     // Block expression or map
            TokenType::LeftBracket => todo!(),   // Array literal
            TokenType::Identifier(_) => todo!(), // Variable or function call
            TokenType::Int(_) => todo!(),        // Int literal
            TokenType::Float(_) => todo!(),      // Float literal
            TokenType::StringLit(_) => todo!(),  // String literal

            _ => return Err(ParserError::UnexpectedToken(self.peek().clone())),
        }
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

    pub fn parse_optional_primitive(&self, name: &str) -> Option<PrimitiveType> {
        return match name {
            "i8" => Some(PrimitiveType::Int8),
            "i16" => Some(PrimitiveType::Int16),
            "i32" => Some(PrimitiveType::Int32),
            "i64" => Some(PrimitiveType::Int64),
            "u8" => Some(PrimitiveType::Uint8),
            "u16" => Some(PrimitiveType::Uint16),
            "u32" => Some(PrimitiveType::Uint32),
            "u64" => Some(PrimitiveType::Uint64),
            "f32" => Some(PrimitiveType::Float32),
            "f64" => Some(PrimitiveType::Float64),
            "str" => Some(PrimitiveType::String),
            "char" => Some(PrimitiveType::Char),
            "bool" => Some(PrimitiveType::Bool),
            "null" => Some(PrimitiveType::Null),
            _ => return None,
        };
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
                TypeExpr::Primitive(PrimitiveType::Int64),
                TypeExpr::Primitive(PrimitiveType::String),
            ]),
        };

        assert_eq!(program.items.len(), 1);
        assert_eq!(program.items[0], Item::TypeAlias(expected_alias));
    }

    #[test]
    #[ignore = "depends on unimplemented parse_expr"]
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
            return_type: Some(TypeExpr::Primitive(PrimitiveType::Int64)),
            params: vec![
                ParamDecl {
                    name: String::from("a"),
                    ty: TypeExpr::Primitive(PrimitiveType::Int64),
                },
                ParamDecl {
                    name: String::from("b"),
                    ty: TypeExpr::Primitive(PrimitiveType::Int64),
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
        // Simulamos: : Movable + Drawable
        let tokens = TokenListBuilder::new()
            .colon()
            .space()
            .identifier("Movable")
            .plus()
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
            ty: TypeExpr::Primitive(PrimitiveType::Int32),
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
                ty: TypeExpr::Primitive(PrimitiveType::String),
            }],
            methods: vec![],
            implements: vec![],
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

    #[test]
    fn test_parse_type_expr_primitive() {
        // i32 should parse as Primitive(Int32)
        let tokens = TokenListBuilder::new()
            .identifier("i32")
            .eof()
            .build();
        let mut parser = Parser::new(tokens);
        let ty = parser.parse_type_expr().unwrap();
        assert_eq!(ty, TypeExpr::Primitive(PrimitiveType::Int32));
    }

    #[test]
    fn test_parse_type_expr_named_with_type_args() {
        // List<i32>
        let tokens = TokenListBuilder::new()
            .identifier("List")
            .lt()
            .identifier("i32")
            .gt()
            .eof()
            .build();
        let mut parser = Parser::new(tokens);
        let ty = parser.parse_type_expr().unwrap();
        assert_eq!(
            ty,
            TypeExpr::Named(
                "List".to_string(),
                vec![TypeExpr::Primitive(PrimitiveType::Int32)]
            )
        );
    }

    #[test]
    fn test_parse_type_expr_function_type() {
        // (i32, str) -> bool
        let tokens = TokenListBuilder::new()
            .left_paren()
            .identifier("i32")
            .comma()
            .space()
            .identifier("str")
            .right_paren()
            .minus()
            .gt()
            .identifier("bool")
            .eof()
            .build();
        let mut parser = Parser::new(tokens);
        let ty = parser.parse_type_expr().unwrap();
        assert_eq!(
            ty,
            TypeExpr::Function(
                vec![
                    TypeExpr::Primitive(PrimitiveType::Int32),
                    TypeExpr::Primitive(PrimitiveType::String),
                ],
                Box::new(TypeExpr::Primitive(PrimitiveType::Bool))
            )
        );
    }

    #[test]
    fn test_parse_type_alias_with_generics() {
        // type Option<T> = T | null;
        let tokens = TokenListBuilder::new()
            .kw_type()
            .space()
            .identifier("Option")
            .lt()
            .identifier("T")
            .gt()
            .space()
            .eq()
            .space()
            .identifier("T")
            .space()
            .pipe()
            .space()
            .kw_null()
            .semicolon()
            .eof()
            .build();
        let mut parser = Parser::new(tokens);
        let result = parser.parse_type_decl().unwrap();
        assert_eq!(
            result,
            Item::TypeAlias(TypeAliasDecl {
                name: "Option".to_string(),
                generics: vec![GenericParam {
                    name: "T".to_string(),
                    bounds: vec![],
                }],
                ty: TypeExpr::Union(vec![
                    TypeExpr::Named("T".to_string(), vec![]),
                    TypeExpr::Primitive(PrimitiveType::Null),
                ]),
            })
        );
    }

    #[test]
    fn test_parse_struct_with_generics() {
        // struct Box<T> { value: T }
        let tokens = TokenListBuilder::new()
            .kw_struct()
            .space()
            .identifier("Box")
            .lt()
            .identifier("T")
            .gt()
            .space()
            .left_brace()
            .space()
            .identifier("value")
            .colon()
            .space()
            .identifier("T")
            .space()
            .right_brace()
            .eof()
            .build();
        let mut parser = Parser::new(tokens);
        let result = parser.parse_struct_decl().unwrap();
        assert_eq!(result.name, "Box");
        assert_eq!(result.generics.len(), 1);
        assert_eq!(result.generics[0].name, "T");
        assert_eq!(result.fields.len(), 1);
        assert_eq!(result.fields[0].name, "value");
        assert_eq!(
            result.fields[0].ty,
            TypeExpr::Named("T".to_string(), vec![])
        );
    }

    #[test]
    fn test_parse_function_no_body() {
        // function foo(x: i32): bool;
        let tokens = TokenListBuilder::new()
            .kw_function()
            .space()
            .identifier("foo")
            .left_paren()
            .identifier("x")
            .colon()
            .space()
            .identifier("i32")
            .right_paren()
            .colon()
            .space()
            .identifier("bool")
            .semicolon()
            .eof()
            .build();
        let mut parser = Parser::new(tokens);
        let result = parser.parse_function_decl().unwrap();
        assert_eq!(
            result,
            FunctionDecl {
                name: "foo".to_string(),
                has_self_param: false,
                generics: vec![],
                return_type: Some(TypeExpr::Primitive(PrimitiveType::Bool)),
                params: vec![ParamDecl {
                    name: "x".to_string(),
                    ty: TypeExpr::Primitive(PrimitiveType::Int32),
                }],
                body: None,
            }
        );
    }

    #[test]
    fn test_parse_function_no_params() {
        // function noop();
        let tokens = TokenListBuilder::new()
            .kw_function()
            .space()
            .identifier("noop")
            .left_paren()
            .right_paren()
            .semicolon()
            .eof()
            .build();
        let mut parser = Parser::new(tokens);
        let result = parser.parse_function_decl().unwrap();
        assert_eq!(result.name, "noop");
        assert_eq!(result.params.len(), 0);
        assert!(result.body.is_none());
        assert!(result.return_type.is_none());
    }

    #[test]
    fn test_parse_struct_with_method() {
        // struct Foo { function bar(): i32; }
        let tokens = TokenListBuilder::new()
            .kw_struct()
            .space()
            .identifier("Foo")
            .space()
            .left_brace()
            .space()
            .kw_function()
            .space()
            .identifier("bar")
            .left_paren()
            .right_paren()
            .colon()
            .space()
            .identifier("i32")
            .semicolon()
            .space()
            .right_brace()
            .eof()
            .build();
        let mut parser = Parser::new(tokens);
        let result = parser.parse_struct_decl().unwrap();
        assert_eq!(result.name, "Foo");
        assert_eq!(result.fields.len(), 0);
        assert_eq!(result.methods.len(), 1);
        assert_eq!(result.methods[0].name, "bar");
        assert_eq!(
            result.methods[0].return_type,
            Some(TypeExpr::Primitive(PrimitiveType::Int32))
        );
    }

    #[test]
    fn test_parse_error_unexpected_token_at_item() {
        let tokens = TokenListBuilder::new()
            .int("42")
            .eof()
            .build();
        let mut parser = Parser::new(tokens);
        assert!(parser.parse_item().is_err());
    }

    #[test]
    fn test_parse_struct_with_multiple_fields() {
        // struct Point { x: f64 y: f64 }
        let tokens = TokenListBuilder::new()
            .kw_struct()
            .space()
            .identifier("Point")
            .space()
            .left_brace()
            .space()
            .identifier("x")
            .colon()
            .space()
            .identifier("f64")
            .space()
            .identifier("y")
            .colon()
            .space()
            .identifier("f64")
            .space()
            .right_brace()
            .eof()
            .build();
        let mut parser = Parser::new(tokens);
        let result = parser.parse_struct_decl().unwrap();
        assert_eq!(result.fields.len(), 2);
        assert_eq!(result.fields[0].name, "x");
        assert_eq!(result.fields[0].ty, TypeExpr::Primitive(PrimitiveType::Float64));
        assert_eq!(result.fields[1].name, "y");
        assert_eq!(result.fields[1].ty, TypeExpr::Primitive(PrimitiveType::Float64));
    }

    #[test]
    fn test_parse_extend_with_method() {
        // extend Foo { function baz(): bool; }
        let tokens = TokenListBuilder::new()
            .kw_extend()
            .space()
            .identifier("Foo")
            .space()
            .left_brace()
            .space()
            .kw_function()
            .space()
            .identifier("baz")
            .left_paren()
            .right_paren()
            .colon()
            .space()
            .identifier("bool")
            .semicolon()
            .space()
            .right_brace()
            .eof()
            .build();
        let mut parser = Parser::new(tokens);
        let result = parser.parse_extend_decl().unwrap();
        if let Item::Extend(ext) = result {
            assert_eq!(ext.methods.len(), 1);
            assert_eq!(ext.methods[0].name, "baz");
        } else {
            panic!("Expected Item::Extend");
        }
    }

    #[test]
    fn test_parse_generic_params_single() {
        // <T>
        let tokens = TokenListBuilder::new()
            .lt()
            .identifier("T")
            .gt()
            .eof()
            .build();
        let mut parser = Parser::new(tokens);
        let generics = parser.parse_generic_params().unwrap();
        assert_eq!(generics.len(), 1);
        assert_eq!(generics[0].name, "T");
    }

    #[test]
    fn test_parse_generic_params_empty() {
        // no < at all
        let tokens = TokenListBuilder::new()
            .identifier("Foo")
            .eof()
            .build();
        let mut parser = Parser::new(tokens);
        let generics = parser.parse_generic_params().unwrap();
        assert_eq!(generics.len(), 0);
    }

    #[test]
    fn test_parse_implements_empty() {
        // no : at all
        let tokens = TokenListBuilder::new()
            .left_brace()
            .eof()
            .build();
        let mut parser = Parser::new(tokens);
        let implements = parser.parse_implements().unwrap();
        assert_eq!(implements.len(), 0);
    }
}
