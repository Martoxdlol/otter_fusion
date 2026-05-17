use std::fmt;

use crate::{
    ast::{
        BinaryOperator, Block, Expr, ExtendDecl, ExternParam, FieldDecl, FunctionDecl,
        GenericParam, InterfaceDecl, Item, ItemKind, Literal, ParamDecl, PrimitiveType, Program,
        Statement, StructDecl, TypeAliasDecl, TypeExpr, UnaryOperator,
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

impl ParserError {
    pub fn span(&self) -> (usize, usize) {
        match self {
            ParserError::UnexpectedToken(tok) => (tok.line, tok.column),
        }
    }
}

impl fmt::Display for ParserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParserError::UnexpectedToken(tok) => {
                write!(f, "unexpected token {}", tok.token_type)
            }
        }
    }
}

impl std::error::Error for ParserError {}

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
        let span = self.peek().span();
        match self.peek().token_type.clone() {
            TokenType::Type => self.parse_type_decl(),
            TokenType::Struct => Ok(Item {
                kind: ItemKind::Struct(self.parse_struct_decl(false)?),
                span,
            }),
            TokenType::Interface => self.parse_interface_decl(),
            TokenType::Function => Ok(Item {
                kind: ItemKind::Function(self.parse_function_decl(false)?),
                span,
            }),
            TokenType::Extend => self.parse_extend_decl(),
            TokenType::Extern => {
                self.advance();
                let next_span = self.peek().span();
                match self.peek().token_type.clone() {
                    TokenType::Struct => Ok(Item {
                        kind: ItemKind::Struct(self.parse_struct_decl(true)?),
                        span: next_span,
                    }),
                    TokenType::Function => Ok(Item {
                        kind: ItemKind::Function(self.parse_function_decl(true)?),
                        span: next_span,
                    }),
                    _ => Err(ParserError::UnexpectedToken(self.peek().clone())),
                }
            }
            _ => Err(ParserError::UnexpectedToken(self.peek().clone())),
        }
    }

    // Items
    pub fn parse_type_decl(&mut self) -> Result<Item, ParserError> {
        let span = self.peek().span();
        self.expect(TokenType::Type)?;
        let name = self.expect_identifier()?;
        let generics = self.parse_generic_params()?;
        self.expect(TokenType::Eq)?;
        let ty = self.parse_type_expr()?;
        self.expect(TokenType::Semicolon)?;
        Ok(Item {
            kind: ItemKind::TypeAlias(TypeAliasDecl { name, generics, ty }),
            span,
        })
    }

    pub fn parse_struct_decl(&mut self, is_extern: bool) -> Result<StructDecl, ParserError> {
        self.expect(TokenType::Struct)?;
        let name = self.expect_identifier()?;
        let generics = self.parse_generic_params()?;
        let implements = self.parse_implements()?;
        let (fields, methods) = self.parse_struct_body(is_extern)?;
        Ok(StructDecl {
            name,
            is_extern,
            fields,
            generics,
            implements,
            methods,
        })
    }

    pub fn parse_interface_decl(&mut self) -> Result<Item, ParserError> {
        let span = self.peek().span();
        self.expect(TokenType::Interface)?;
        let name = self.expect_identifier()?;
        let generics = self.parse_generic_params()?;
        let implements = self.parse_implements()?;
        let (fields, methods) = self.parse_struct_body(false)?;
        Ok(Item {
            kind: ItemKind::Interface(InterfaceDecl {
                name,
                generics,
                implements,
                fields,
                methods,
            }),
            span,
        })
    }

    pub fn parse_function_decl(&mut self, is_extern: bool) -> Result<FunctionDecl, ParserError> {
        let span = self.peek().span();
        self.expect(TokenType::Function)?;
        let name = self.expect_identifier()?;
        let generics = self.parse_generic_params()?;
        let mut params = self.parse_function_args(is_extern)?;
        let mut return_type = None;
        if self.expect_optional(TokenType::Colon) {
            return_type = Some(self.parse_return_type(is_extern)?);
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
            is_extern,
            generics,
            return_type,
            params,
            body,
            span,
        })
    }

    fn parse_return_type(&mut self, in_extern: bool) -> Result<TypeExpr, ParserError> {
        if in_extern {
            self.parse_extern_type_expr()
        } else {
            self.parse_type_expr()
        }
    }

    // In extern contexts, a type expression can include *T (pointer) atoms and
    // *T | null unions. We parse a regular type_expr but allow pointer atoms.
    fn parse_extern_type_expr(&mut self) -> Result<TypeExpr, ParserError> {
        let ty = self.parse_extern_type_atom()?;

        if self.peek().token_type == TokenType::Pipe {
            let mut variants = vec![ty];
            while self.expect_optional(TokenType::Pipe) {
                variants.push(self.parse_extern_type_atom()?);
            }
            Ok(TypeExpr::Union(variants))
        } else {
            Ok(ty)
        }
    }

    fn parse_extern_type_atom(&mut self) -> Result<TypeExpr, ParserError> {
        if self.peek().token_type == TokenType::Star {
            self.advance();
            let inner = self.parse_type_atom()?;
            Ok(TypeExpr::Pointer(Box::new(inner)))
        } else {
            self.parse_type_atom()
        }
    }

    fn parse_extern_fn_type_param(&mut self) -> Result<ExternParam, ParserError> {
        let name = self.expect_identifier()?;
        self.expect(TokenType::Colon)?;
        let is_pointer = self.expect_optional(TokenType::Star);
        let ty = self.parse_type_expr()?;
        Ok(ExternParam {
            name,
            ty,
            is_pointer,
        })
    }

    pub fn parse_extend_decl(&mut self) -> Result<Item, ParserError> {
        let span = self.peek().span();
        self.expect(TokenType::Extend)?;
        let generics = self.parse_generic_params()?;
        let name = self.expect_identifier()?;
        let type_args = self.parse_type_args()?;
        let implements = self.parse_implements()?;

        let methods = self.parse_extend_body()?;

        Ok(Item {
            kind: ItemKind::Extend(ExtendDecl {
                target: TypeExpr::Named(name, type_args),
                generic_params: generics,
                implements,
                methods,
            }),
            span,
        })
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
            // Extern function type: extern (name: T, name: *T) => return_type
            TokenType::Extern => {
                self.advance();
                self.expect(TokenType::LeftParen)?;
                let mut params = Vec::new();
                if self.peek().token_type != TokenType::RightParen {
                    params.push(self.parse_extern_fn_type_param()?);
                    while self.expect_optional(TokenType::Comma) {
                        params.push(self.parse_extern_fn_type_param()?);
                    }
                }
                self.expect(TokenType::RightParen)?;
                self.expect(TokenType::FatArrow)?;
                let return_type = self.parse_extern_type_expr()?;
                Ok(TypeExpr::ExternFunction(params, Box::new(return_type)))
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

    pub fn parse_field(&mut self, in_extern: bool) -> Result<FieldDecl, ParserError> {
        let name = self.expect_identifier()?;
        self.expect(TokenType::Colon)?;
        let (ty, is_pointer) = self.parse_maybe_pointer_type(in_extern)?;
        Ok(FieldDecl {
            name,
            ty,
            is_pointer,
        })
    }

    // Struct
    pub fn parse_struct_body(
        &mut self,
        in_extern: bool,
    ) -> Result<(Vec<FieldDecl>, Vec<FunctionDecl>), ParserError> {
        self.expect(TokenType::LeftBrace)?;

        // Implement struct body parsing logic here
        let mut fields: Vec<FieldDecl> = Vec::new();
        let mut methods: Vec<FunctionDecl> = Vec::new();

        loop {
            let tok = self.peek();
            match &tok.token_type {
                TokenType::Identifier(_) => fields.push(self.parse_field(in_extern)?),
                TokenType::Function => methods.push(self.parse_function_decl(false)?),
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
                TokenType::Function => methods.push(self.parse_function_decl(false)?),
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

    pub fn parse_function_args(&mut self, in_extern: bool) -> Result<Vec<ParamDecl>, ParserError> {
        self.expect(TokenType::LeftParen)?;
        let mut params = Vec::new();

        if self.peek().token_type != TokenType::RightParen {
            params.push(self.parse_param_decl(in_extern)?);
            while self.expect_optional(TokenType::Comma) {
                params.push(self.parse_param_decl(in_extern)?);
            }
        }

        self.expect(TokenType::RightParen)?;
        Ok(params)
    }

    pub fn parse_param_decl(&mut self, in_extern: bool) -> Result<ParamDecl, ParserError> {
        let name = self.expect_identifier()?;
        self.expect(TokenType::Colon)?;
        let (ty, is_pointer) = self.parse_maybe_pointer_type(in_extern)?;
        Ok(ParamDecl {
            name,
            ty,
            is_pointer,
        })
    }

    // In extern contexts, a field/param type may be prefixed with `*` to mark
    // it as a pointer. Outside extern contexts, `*` is not allowed here.
    fn parse_maybe_pointer_type(
        &mut self,
        in_extern: bool,
    ) -> Result<(TypeExpr, bool), ParserError> {
        if in_extern && self.peek().token_type == TokenType::Star {
            self.advance();
            let ty = self.parse_type_expr()?;
            Ok((ty, true))
        } else if in_extern {
            let ty = self.parse_extern_type_expr()?;
            Ok((ty, false))
        } else {
            let ty = self.parse_type_expr()?;
            Ok((ty, false))
        }
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
            if self.peek().token_type == TokenType::RightBrace {
                self.advance();
                break;
            }

            let stmt = self.parse_statement()?;

            if self.expect_optional(TokenType::Semicolon) {
                // Semicolon present — it's a regular statement, continue looping
                statements.push(stmt);
            } else if self.peek().token_type == TokenType::RightBrace {
                // No semicolon, followed by } — final return expression
                if let Statement::Expr(expr) = stmt {
                    final_expression = Some(expr);
                } else {
                    statements.push(stmt);
                }
                self.advance();
                break;
            } else {
                return Err(ParserError::UnexpectedToken(self.peek().clone()));
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
            TokenType::Break => {
                self.expect(TokenType::Break)?;
                Ok(Statement::Break)
            }
            TokenType::Continue => {
                self.expect(TokenType::Continue)?;
                Ok(Statement::Continue)
            }
            _ => Ok(Statement::Expr(self.parse_expr()?)),
        }
    }

    // - base:
    // literal (int, float, string, char, bool, null)
    // variable
    //
    // - recursive:
    // (expr)                              — grouping
    // { stmt; stmt; expr }                — block expression
    // [expr, ...]                         — list literal
    // { "key": value, ... }               — map literal
    // Type { field: value, ... }          — struct init
    // (params): Type { body }             — function literal
    // if cond { ... } else { ... }        — if expression
    // expr.member                         — member access
    // expr<T>(args)                       — function call
    // op expr                             — unary operation
    // expr op expr                        — binary operation
    // expr as Type                        — type cast
    // expr is Type                        — type check
    pub fn parse_expr(&mut self) -> Result<Expr, ParserError> {
        self.parse_expr_bp(0)
    }

    fn parse_expr_bp(&mut self, min_bp: u8) -> Result<Expr, ParserError> {
        // 1. Parse prefix (left-hand side)
        let mut lhs = self.parse_prefix()?;

        // 2. Loop: postfix and infix operators
        // Infix -> between two expressions, e.g. a + b
        // Postfix -> after an expression, e.g. a.b, func<T>(args), a as Type
        loop {
            let op: BinaryOperator = match &self.peek().token_type {
                // Postfix: member access
                TokenType::Dot => {
                    self.advance();
                    let member = self.expect_identifier()?;
                    lhs = Expr::Member(Box::new(lhs), member);
                    continue;
                }
                // Postfix: function call
                TokenType::LeftParen => {
                    lhs = self.parse_call_args(lhs, vec![])?;
                    continue;
                }
                // Postfix: as / is
                TokenType::As => {
                    if 15 < min_bp {
                        break;
                    }
                    self.advance();
                    let ty = self.parse_type_expr()?;
                    lhs = Expr::As(Box::new(lhs), ty);
                    continue;
                }
                TokenType::Is => {
                    if 15 < min_bp {
                        break;
                    }
                    self.advance();
                    let ty = self.parse_type_expr()?;
                    lhs = Expr::Is(Box::new(lhs), ty);
                    continue;
                }
                // Infix binary operators
                TokenType::Or => BinaryOperator::Or,
                TokenType::And => BinaryOperator::And,
                TokenType::EqEq => BinaryOperator::Eq,
                TokenType::BangEq => BinaryOperator::Neq,
                TokenType::LT => BinaryOperator::Lt,
                TokenType::GT => BinaryOperator::Gt,
                TokenType::Plus => BinaryOperator::Add,
                TokenType::Minus => BinaryOperator::Sub,
                TokenType::Star => BinaryOperator::Mul,
                TokenType::Slash => BinaryOperator::Div,
                TokenType::Percent => BinaryOperator::Mod,
                _ => break, // not an operator, stop
            };

            let prec = self.infix_precedence(&op);
            if prec < min_bp {
                break;
            }

            // case: x + y + z

            // min_bp: 0
            // lhs: x
            // op: +
            // prec: 5
            // min_bp: 0 (initial call) -> 5 + 1 = 6 (right side of +)
            // recursive call: parse_expr_bp(6)

            // min_bp: 6
            // lhs: y
            // op: +
            // prec: 5
            // prec < min_bp (5 < 6) -> stop, return y as rhs of first +

            // return to first call:
            // lhs: x
            // op: +
            // rhs: y (result of second call)
            // lhs = x + y
            // return to main loop -> (x + y) + z

            //
            // case: x + y * z

            // min_bp: 0
            // lhs: x
            // op: +
            // prec: 5
            // min_bp: 0 (initial call) -> 5 + 1 = 6 (right side of +)
            // recursive call: parse_expr_bp(6)

            // min_bp: 6
            // lhs: y
            // op: *
            // prec: 6
            // min_bp: 6 -> 6 + 1 = 7 (right side of *)
            // recursive call: parse_expr_bp(7)

            // min_bp: 7
            // lhs: z
            // no more operators -> return z
            // return to second call:
            // lhs: y
            // op: *
            // rhs: z

            // return to first call:
            // lhs: x
            // op: +
            // rhs: (y * z)
            // lhs = x + (y * z)
            // return to main loop -> (x + (y * z))

            self.advance(); // consume operator
            let rhs = self.parse_expr_bp(prec + 1)?; // +1 = left-associative
            lhs = Expr::BinaryOp(Box::new(lhs), op, Box::new(rhs));
        }

        Ok(lhs)
    }

    fn parse_prefix(&mut self) -> Result<Expr, ParserError> {
        match &self.peek().token_type {
            // Unary operators
            TokenType::Minus => {
                self.advance();
                let expr = self.parse_expr_bp(13)?; // high bp for prefix
                Ok(Expr::UnaryOp(UnaryOperator::Neg, Box::new(expr)))
            }
            TokenType::Bang => {
                self.advance();
                let expr = self.parse_expr_bp(13)?;
                Ok(Expr::UnaryOp(UnaryOperator::Not, Box::new(expr)))
            }
            // Literals
            TokenType::Int(_) => {
                if let TokenType::Int(v) = &self.peek().token_type {
                    let v = v.clone();
                    self.advance();
                    Ok(Expr::Literal(Literal::Int(v)))
                } else {
                    unreachable!()
                }
            }
            TokenType::Float(_) => {
                if let TokenType::Float(v) = &self.peek().token_type {
                    let v = v.clone();
                    self.advance();
                    Ok(Expr::Literal(Literal::Float(v)))
                } else {
                    unreachable!()
                }
            }
            TokenType::StringLit(_) => {
                if let TokenType::StringLit(v) = &self.peek().token_type {
                    let v = v.clone();
                    self.advance();
                    Ok(Expr::Literal(Literal::String(v)))
                } else {
                    unreachable!()
                }
            }
            TokenType::CharLit(_) => {
                if let TokenType::CharLit(v) = &self.peek().token_type {
                    let v = *v;
                    self.advance();
                    Ok(Expr::Literal(Literal::Char(v)))
                } else {
                    unreachable!()
                }
            }
            TokenType::True => {
                self.advance();
                Ok(Expr::Literal(Literal::Bool(true)))
            }
            TokenType::False => {
                self.advance();
                Ok(Expr::Literal(Literal::Bool(false)))
            }
            TokenType::Null => {
                self.advance();
                Ok(Expr::Literal(Literal::Null))
            }

            // Parenthesized expr or function literal
            TokenType::LeftParen => self.parse_paren_or_fn_literal(),

            // Array literal
            TokenType::LeftBracket => self.parse_list_literal(),

            // Block or map literal
            TokenType::LeftBrace => self.parse_block_or_map(),

            // Identifier: variable
            TokenType::Identifier(name) => {
                let name = name.clone();
                self.advance();
                Ok(Expr::Variable(name))
            }

            TokenType::If => self.parse_if_expr(),

            _ => Err(ParserError::UnexpectedToken(self.peek().clone())),
        }
    }

    fn infix_precedence(&self, op: &BinaryOperator) -> u8 {
        match op {
            BinaryOperator::Or => 1,
            BinaryOperator::And => 2,
            BinaryOperator::Eq | BinaryOperator::Neq => 3,
            BinaryOperator::Lt | BinaryOperator::Le | BinaryOperator::Gt | BinaryOperator::Ge => 4,
            BinaryOperator::Add | BinaryOperator::Sub => 5,
            BinaryOperator::Mul | BinaryOperator::Div | BinaryOperator::Mod => 6,
        }
    }

    fn parse_call_args(
        &mut self,
        callee: Expr,
        type_args: Vec<TypeExpr>,
    ) -> Result<Expr, ParserError> {
        self.expect(TokenType::LeftParen)?;
        let mut args = Vec::new();

        if self.peek().token_type != TokenType::RightParen {
            args.push(self.parse_expr()?);
            while self.expect_optional(TokenType::Comma) {
                args.push(self.parse_expr()?);
            }
        }

        self.expect(TokenType::RightParen)?;
        Ok(Expr::Call(Box::new(callee), type_args, args))
    }

    fn parse_paren_or_fn_literal(&mut self) -> Result<Expr, ParserError> {
        self.expect(TokenType::LeftParen)?;
        let expr = self.parse_expr()?;
        self.expect(TokenType::RightParen)?;
        Ok(expr)
    }

    fn parse_list_literal(&mut self) -> Result<Expr, ParserError> {
        self.expect(TokenType::LeftBracket)?;
        let mut elements = Vec::new();

        if self.peek().token_type != TokenType::RightBracket {
            elements.push(self.parse_expr()?);
            while self.expect_optional(TokenType::Comma) {
                if self.peek().token_type == TokenType::RightBracket {
                    break;
                }
                elements.push(self.parse_expr()?);
            }
        }

        self.expect(TokenType::RightBracket)?;
        Ok(Expr::LiteralList(elements))
    }

    fn parse_if_expr(&mut self) -> Result<Expr, ParserError> {
        self.expect(TokenType::If)?;
        let cond = self.parse_expr()?;
        let then_branch = self.parse_block()?;
        let else_branch = if self.expect_optional(TokenType::Else) {
            Some(Box::new(self.parse_block()?))
        } else {
            None
        };
        Ok(Expr::If(Box::new(cond), Box::new(then_branch), else_branch))
    }
    fn parse_block_or_map(&mut self) -> Result<Expr, ParserError> {
        self.expect(TokenType::LeftBrace)?;
        if self.peek().token_type == TokenType::RightBrace {
            self.advance();
            return Ok(Expr::LiteralMap(vec![]));
        }

        // if token is string literal followed by colon, it's a map literal
        // match StringLit, Colon
        // Can be other lit types
        let is_map = self.check_is_map();

        if is_map {
            self.parse_map_literal()
        } else {
            let block = self.parse_block()?;
            Ok(Expr::Block(Box::new(block)))
        }
    }

    fn check_is_map(&mut self) -> bool {
        let tok1 = self.peek();
        match tok1.token_type {
            TokenType::StringLit(_)
            | TokenType::Int(_)
            | TokenType::Float(_)
            | TokenType::CharLit(_) => {}
            _ => return false,
        };

        self.advance();
        let tok2 = self.peek().clone();
        self.back();

        return tok2.token_type == TokenType::Colon;
    }

    fn parse_map_literal(&mut self) -> Result<Expr, ParserError> {
        let mut entries = Vec::new();

        loop {
            let key = match &self.peek().token_type {
                TokenType::StringLit(s) => {
                    let s = Expr::Literal(Literal::String(s.clone()));
                    self.advance();
                    s
                }
                TokenType::Int(i) => {
                    let s = Expr::Literal(Literal::Int(i.clone()));
                    self.advance();
                    s
                }
                TokenType::Float(f) => {
                    let s = Expr::Literal(Literal::Float(f.clone()));
                    self.advance();
                    s
                }
                TokenType::CharLit(c) => {
                    let s = Expr::Literal(Literal::Char(*c));
                    self.advance();
                    s
                }
                _ => return Err(ParserError::UnexpectedToken(self.peek().clone())),
            };

            self.expect(TokenType::Colon)?;
            let value = self.parse_expr()?;
            entries.push((key, value));

            if !self.expect_optional(TokenType::Comma) {
                break;
            }
        }

        self.expect(TokenType::RightBrace)?;
        Ok(Expr::LiteralMap(entries))
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

    pub fn back(&mut self) {
        if self.current > 0 {
            self.current -= 1;
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
