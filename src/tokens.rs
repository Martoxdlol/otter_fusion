#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Token {
    pub token_type: TokenType,
    pub position: usize,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TokenType {
    // Single-character delimiters
    LeftParen,    // (
    RightParen,   // )
    LeftBrace,    // {
    RightBrace,   // }
    LeftBracket,  // [
    RightBracket, // ]

    // Operators
    LT,        // <
    GT,        // >
    Eq,        // =
    EqEq,      // ==
    Bang,      // !
    BangEq,    // !=
    Plus,      // +
    Minus,     // -
    Star,      // *
    Slash,     // /
    Percent,   // %
    And,       // &&
    Or,        // ||
    Pipe,      // |
    Colon,     // :
    Semicolon, // ;
    Dot,       // .
    Comma,     // ,

    // Literals
    Identifier(String), // variable/function name
    StringLit(String),  // "texto entre comillas"
    CharLit(char),      // 'a'
    Float(String),      // 123.0, 3.14
    Int(String),        // 123

    // Other
    Comment(String), // // comment

    // Keywords
    Struct,    // struct
    Function,  // function
    For,       // for
    While,     // while
    Null,      // null
    True,      // true
    False,     // false
    Var,       // var
    If,        // if
    Else,      // else
    Extend,    // extend
    Return,    // return
    Interface, // interface
    Is,        // is
    In,        // in
    Type,      // type
    As,        // as
    SelfRef,   // self
    Match,     // match
    Class,     // class
    Continue,  // continue
    Break,     // break

    EOF,
}

impl Token {
    pub fn new(token_type: TokenType, position: usize, line: usize, column: usize) -> Self {
        Self {
            token_type,
            position,
            line,
            column,
        }
    }

    pub fn left_paren(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::LeftParen, position, line, column)
    }

    pub fn right_paren(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::RightParen, position, line, column)
    }

    pub fn left_brace(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::LeftBrace, position, line, column)
    }

    pub fn right_brace(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::RightBrace, position, line, column)
    }

    pub fn left_bracket(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::LeftBracket, position, line, column)
    }

    pub fn right_bracket(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::RightBracket, position, line, column)
    }

    pub fn lt(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::LT, position, line, column)
    }

    pub fn gt(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::GT, position, line, column)
    }

    pub fn eq(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::Eq, position, line, column)
    }

    pub fn eq_eq(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::EqEq, position, line, column)
    }

    pub fn bang(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::Bang, position, line, column)
    }

    pub fn bang_eq(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::BangEq, position, line, column)
    }

    pub fn plus(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::Plus, position, line, column)
    }

    pub fn minus(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::Minus, position, line, column)
    }

    pub fn star(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::Star, position, line, column)
    }

    pub fn slash(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::Slash, position, line, column)
    }

    pub fn percent(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::Percent, position, line, column)
    }

    pub fn and(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::And, position, line, column)
    }

    pub fn or(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::Or, position, line, column)
    }

    pub fn pipe(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::Pipe, position, line, column)
    }

    pub fn colon(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::Colon, position, line, column)
    }

    pub fn semicolon(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::Semicolon, position, line, column)
    }

    pub fn dot(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::Dot, position, line, column)
    }

    pub fn comma(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::Comma, position, line, column)
    }

    pub fn identifier(name: String, position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::Identifier(name), position, line, column)
    }

    pub fn string_lit(value: String, position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::StringLit(value), position, line, column)
    }

    pub fn char_lit(value: char, position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::CharLit(value), position, line, column)
    }

    pub fn float(value: String, position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::Float(value), position, line, column)
    }

    pub fn int(value: String, position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::Int(value), position, line, column)
    }

    pub fn comment(value: String, position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::Comment(value), position, line, column)
    }

    pub fn keyword(keyword: TokenType, position: usize, line: usize, column: usize) -> Self {
        Self::new(keyword, position, line, column)
    }

    pub fn eof(position: usize, line: usize, column: usize) -> Self {
        Self::new(TokenType::EOF, position, line, column)
    }
}

impl TokenType {
    pub fn text_len(&self) -> usize {
        match self {
            // Single-character
            TokenType::LeftParen
            | TokenType::RightParen
            | TokenType::LeftBrace
            | TokenType::RightBrace
            | TokenType::LeftBracket
            | TokenType::RightBracket
            | TokenType::LT
            | TokenType::GT
            | TokenType::Eq
            | TokenType::Bang
            | TokenType::Plus
            | TokenType::Minus
            | TokenType::Star
            | TokenType::Slash
            | TokenType::Percent
            | TokenType::Pipe
            | TokenType::Colon
            | TokenType::Semicolon
            | TokenType::Dot
            | TokenType::Comma => 1,

            // Two-character
            TokenType::EqEq | TokenType::BangEq | TokenType::And | TokenType::Or => 2,

            // Literals
            TokenType::Identifier(name) => name.len(),
            TokenType::StringLit(value) => value.len() + 2, // quotes
            TokenType::CharLit(_) => 3,                     // 'x'
            TokenType::Float(value) => value.len(),
            TokenType::Int(value) => value.len(),

            // Comment: // + content
            TokenType::Comment(value) => value.len() + 2,

            // Keywords
            TokenType::Struct => 6,
            TokenType::Function => 8,
            TokenType::For => 3,
            TokenType::While => 5,
            TokenType::Null => 4,
            TokenType::True => 4,
            TokenType::False => 5,
            TokenType::Var => 3,
            TokenType::If => 2,
            TokenType::Else => 4,
            TokenType::Extend => 6,
            TokenType::Return => 6,
            TokenType::Interface => 9,
            TokenType::Is => 2,
            TokenType::In => 2,
            TokenType::Type => 4,
            TokenType::As => 2,
            TokenType::SelfRef => 4,
            TokenType::Match => 5,
            TokenType::Class => 5,
            TokenType::Continue => 8,
            TokenType::Break => 5,

            TokenType::EOF => 0,
        }
    }
}

pub struct TokenListBuilder {
    tokens: Vec<Token>,
    position: usize,
    line: usize,
    column: usize,
}

impl TokenListBuilder {
    pub fn new() -> Self {
        Self {
            tokens: Vec::new(),
            position: 0,
            line: 1,
            column: 1,
        }
    }

    fn push(mut self, token_type: TokenType) -> Self {
        let len = token_type.text_len();
        self.tokens.push(Token::new(
            token_type,
            self.position,
            self.line,
            self.column,
        ));
        self.position += len;
        self.column += len;
        self
    }

    pub fn space(mut self) -> Self {
        self.position += 1;
        self.column += 1;
        self
    }

    pub fn spaces(mut self, n: usize) -> Self {
        self.position += n;
        self.column += n;
        self
    }

    pub fn newline(mut self) -> Self {
        self.position += 1;
        self.line += 1;
        self.column = 1;
        self
    }

    // Delimiters
    pub fn left_paren(self) -> Self {
        self.push(TokenType::LeftParen)
    }
    pub fn right_paren(self) -> Self {
        self.push(TokenType::RightParen)
    }
    pub fn left_brace(self) -> Self {
        self.push(TokenType::LeftBrace)
    }
    pub fn right_brace(self) -> Self {
        self.push(TokenType::RightBrace)
    }
    pub fn left_bracket(self) -> Self {
        self.push(TokenType::LeftBracket)
    }
    pub fn right_bracket(self) -> Self {
        self.push(TokenType::RightBracket)
    }

    // Operators
    pub fn lt(self) -> Self {
        self.push(TokenType::LT)
    }
    pub fn gt(self) -> Self {
        self.push(TokenType::GT)
    }
    pub fn eq(self) -> Self {
        self.push(TokenType::Eq)
    }
    pub fn eq_eq(self) -> Self {
        self.push(TokenType::EqEq)
    }
    pub fn bang(self) -> Self {
        self.push(TokenType::Bang)
    }
    pub fn bang_eq(self) -> Self {
        self.push(TokenType::BangEq)
    }
    pub fn plus(self) -> Self {
        self.push(TokenType::Plus)
    }
    pub fn minus(self) -> Self {
        self.push(TokenType::Minus)
    }
    pub fn star(self) -> Self {
        self.push(TokenType::Star)
    }
    pub fn slash(self) -> Self {
        self.push(TokenType::Slash)
    }
    pub fn percent(self) -> Self {
        self.push(TokenType::Percent)
    }
    pub fn and(self) -> Self {
        self.push(TokenType::And)
    }
    pub fn or(self) -> Self {
        self.push(TokenType::Or)
    }
    pub fn pipe(self) -> Self {
        self.push(TokenType::Pipe)
    }
    pub fn colon(self) -> Self {
        self.push(TokenType::Colon)
    }
    pub fn semicolon(self) -> Self {
        self.push(TokenType::Semicolon)
    }
    pub fn dot(self) -> Self {
        self.push(TokenType::Dot)
    }
    pub fn comma(self) -> Self {
        self.push(TokenType::Comma)
    }

    // Literals
    pub fn identifier(self, name: &str) -> Self {
        self.push(TokenType::Identifier(name.to_string()))
    }
    pub fn string_lit(self, value: &str) -> Self {
        self.push(TokenType::StringLit(value.to_string()))
    }
    pub fn char_lit(self, value: char) -> Self {
        self.push(TokenType::CharLit(value))
    }
    pub fn float(self, value: &str) -> Self {
        self.push(TokenType::Float(value.to_string()))
    }
    pub fn int(self, value: &str) -> Self {
        self.push(TokenType::Int(value.to_string()))
    }

    // Other
    pub fn comment(self, value: &str) -> Self {
        self.push(TokenType::Comment(value.to_string()))
    }

    // Keywords
    pub fn kw_struct(self) -> Self {
        self.push(TokenType::Struct)
    }
    pub fn kw_function(self) -> Self {
        self.push(TokenType::Function)
    }
    pub fn kw_for(self) -> Self {
        self.push(TokenType::For)
    }
    pub fn kw_while(self) -> Self {
        self.push(TokenType::While)
    }
    pub fn kw_null(self) -> Self {
        self.push(TokenType::Null)
    }
    pub fn kw_true(self) -> Self {
        self.push(TokenType::True)
    }
    pub fn kw_false(self) -> Self {
        self.push(TokenType::False)
    }
    pub fn kw_var(self) -> Self {
        self.push(TokenType::Var)
    }
    pub fn kw_extend(self) -> Self {
        self.push(TokenType::Extend)
    }
    pub fn kw_return(self) -> Self {
        self.push(TokenType::Return)
    }
    pub fn kw_interface(self) -> Self {
        self.push(TokenType::Interface)
    }
    pub fn kw_is(self) -> Self {
        self.push(TokenType::Is)
    }
    pub fn kw_type(self) -> Self {
        self.push(TokenType::Type)
    }
    pub fn kw_as(self) -> Self {
        self.push(TokenType::As)
    }
    pub fn kw_self(self) -> Self {
        self.push(TokenType::SelfRef)
    }
    pub fn kw_match(self) -> Self {
        self.push(TokenType::Match)
    }
    pub fn kw_class(self) -> Self {
        self.push(TokenType::Class)
    }
    pub fn kw_continue(self) -> Self {
        self.push(TokenType::Continue)
    }
    pub fn kw_break(self) -> Self {
        self.push(TokenType::Break)
    }

    pub fn eof(self) -> Self {
        self.push(TokenType::EOF)
    }

    pub fn build(self) -> Vec<Token> {
        self.tokens
    }
}
