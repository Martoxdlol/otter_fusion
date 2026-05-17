use std::fmt;

use crate::tokens::{Token, TokenType};

pub struct Lexer {
    input: Vec<char>,
    position: usize,

    column: usize,
    line: usize,
}

fn char_is_identifier_starter(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

fn char_is_identifier(c: char) -> bool {
    char_is_identifier_starter(c) || c.is_ascii_digit()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LexerError {
    UnexpectedCharacter(char, usize, usize),
    UnterminatedString(usize, usize),
    InvalidNumber(String, usize, usize),
}

impl LexerError {
    pub fn span(&self) -> (usize, usize) {
        match self {
            LexerError::UnexpectedCharacter(_, line, col)
            | LexerError::UnterminatedString(line, col)
            | LexerError::InvalidNumber(_, line, col) => (*line, *col),
        }
    }
}

impl fmt::Display for LexerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LexerError::UnexpectedCharacter(c, _, _) => write!(f, "unexpected character '{c}'"),
            LexerError::UnterminatedString(_, _) => write!(f, "unterminated string literal"),
            LexerError::InvalidNumber(n, _, _) => write!(f, "invalid number literal '{n}'"),
        }
    }
}

impl std::error::Error for LexerError {}

impl Iterator for Lexer {
    type Item = Result<Token, LexerError>;

    fn next(&mut self) -> Option<Self::Item> {
        match Lexer::next_token(self) {
            Some(Ok(token)) => Some(Ok(token)),
            Some(Err(err)) => Some(Err(err)),
            None => None,
        }
    }
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        Self {
            input: input.chars().collect(),
            position: 0,
            column: 1,
            line: 1,
        }
    }

    pub fn next_token(&mut self) -> Option<Result<Token, LexerError>> {
        let mut c = self.peek();

        while let Some(chr) = c {
            if chr.is_whitespace() {
                self.advance();
                c = self.peek();
            } else {
                break;
            }
        }

        if let Some(c) = c {
            if char_is_identifier_starter(c) {
                return Some(Ok(self.scan_literal()));
            } else if c.is_ascii_digit() {
                return Some(self.scan_number());
            } else {
                self.advance();
                return match c {
                    '\'' => Some(self.scan_char()),
                    '"' => Some(self.scan_string()),
                    '/' => {
                        if self.peek() == Some('/') {
                            self.advance();
                            return Some(Ok(self.scan_comment()));
                        } else {
                            return Some(Ok(self.token(TokenType::Slash)));
                        }
                    }
                    '(' => Some(Ok(self.token(TokenType::LeftParen))),
                    ')' => Some(Ok(self.token(TokenType::RightParen))),
                    '{' => Some(Ok(self.token(TokenType::LeftBrace))),
                    '}' => Some(Ok(self.token(TokenType::RightBrace))),
                    '[' => Some(Ok(self.token(TokenType::LeftBracket))),
                    ']' => Some(Ok(self.token(TokenType::RightBracket))),
                    '<' => {
                        if self.peek() == Some('=') {
                            self.advance();
                            return Some(Ok(self.token(TokenType::LtEq)));
                        } else {
                            return Some(Ok(self.token(TokenType::LT)));
                        }
                    }
                    '>' => {
                        if self.peek() == Some('=') {
                            self.advance();
                            return Some(Ok(self.token(TokenType::GtEq)));
                        } else {
                            return Some(Ok(self.token(TokenType::GT)));
                        }
                    }
                    '=' => {
                        if self.peek() == Some('=') {
                            self.advance();
                            return Some(Ok(self.token(TokenType::EqEq)));
                        } else if self.peek() == Some('>') {
                            self.advance();
                            return Some(Ok(self.token(TokenType::FatArrow)));
                        } else {
                            return Some(Ok(self.token(TokenType::Eq)));
                        }
                    }
                    '!' => {
                        if self.peek() == Some('=') {
                            self.advance();
                            return Some(Ok(self.token(TokenType::BangEq)));
                        } else {
                            return Some(Ok(self.token(TokenType::Bang)));
                        }
                    }
                    '+' => Some(Ok(self.token(TokenType::Plus))),
                    '-' => Some(Ok(self.token(TokenType::Minus))),
                    '*' => Some(Ok(self.token(TokenType::Star))),
                    '%' => Some(Ok(self.token(TokenType::Percent))),
                    '&' => {
                        if self.peek() == Some('&') {
                            self.advance();
                            return Some(Ok(self.token(TokenType::And)));
                        } else {
                            return self.err_unexpected('&');
                        }
                    }
                    '|' => {
                        if self.peek() == Some('|') {
                            self.advance();
                            return Some(Ok(self.token(TokenType::Or)));
                        } else {
                            return Some(Ok(self.token(TokenType::Pipe)));
                        }
                    }
                    '.' => Some(Ok(self.token(TokenType::Dot))),
                    ',' => Some(Ok(self.token(TokenType::Comma))),
                    ':' => Some(Ok(self.token(TokenType::Colon))),
                    ';' => Some(Ok(self.token(TokenType::Semicolon))),
                    _ => self.err_unexpected(c),
                };
            }
        }

        Some(Ok(self.token(TokenType::EOF)))
    }

    pub fn scan_all(&mut self) -> Result<Vec<Token>, LexerError> {
        let mut tokens = Vec::new();

        while let Some(result) = self.next_token() {
            match result {
                Ok(token) => {
                    if matches!(token.token_type, TokenType::EOF) {
                        break;
                    }
                    if matches!(token.token_type, TokenType::Comment(_)) {
                        continue;
                    }
                    tokens.push(token);
                }
                Err(err) => return Err(err),
            }
        }

        Ok(tokens)
    }

    fn scan_literal(&mut self) -> Token {
        let mut literal = String::new();
        while let Some(c) = self.peek() {
            if char_is_identifier(c) {
                literal.push(c);
                self.advance();
            } else {
                break;
            }
        }

        match literal.as_str() {
            "struct" => self.token(TokenType::Struct),
            "function" => self.token(TokenType::Function),
            "for" => self.token(TokenType::For),
            "while" => self.token(TokenType::While),
            "null" => self.token(TokenType::Null),
            "true" => self.token(TokenType::True),
            "false" => self.token(TokenType::False),
            "var" => self.token(TokenType::Var),
            "extend" => self.token(TokenType::Extend),
            "return" => self.token(TokenType::Return),
            "interface" => self.token(TokenType::Interface),
            "is" => self.token(TokenType::Is),
            "type" => self.token(TokenType::Type),
            "as" => self.token(TokenType::As),
            "in" => self.token(TokenType::In),
            "self" => self.token(TokenType::SelfRef),
            "match" => self.token(TokenType::Match),
            "class" => self.token(TokenType::Class),
            "if" => self.token(TokenType::If),
            "else" => self.token(TokenType::Else),
            "continue" => self.token(TokenType::Continue),
            "break" => self.token(TokenType::Break),
            "extern" => self.token(TokenType::Extern),
            "import" => self.token(TokenType::Import),
            "from" => self.token(TokenType::From),

            _ => self.token(TokenType::Identifier(literal)),
        }
    }

    fn scan_number(&mut self) -> Result<Token, LexerError> {
        let mut number = String::new();
        let mut has_dot = false;
        let mut last_char_was_dot = false;

        while let Some(c) = self.peek() {
            match c {
                '0'..='9' => {
                    self.advance();
                    number.push(c);
                    last_char_was_dot = false;
                }
                '.' => {
                    if !has_dot {
                        has_dot = true;
                        self.advance();
                        number.push(c);

                        last_char_was_dot = true;
                    } else {
                        return Err(LexerError::InvalidNumber(number, self.line, self.column));
                    }
                }

                _ => break,
            }
        }

        if last_char_was_dot {
            return Err(LexerError::InvalidNumber(number, self.line, self.column));
        }

        if has_dot {
            Ok(self.token(TokenType::Float(number)))
        } else {
            Ok(self.token(TokenType::Int(number)))
        }
    }

    fn scan_string(&mut self) -> Result<Token, LexerError> {
        let mut value = String::new();

        let mut escaped = false;

        while let Some(c) = self.advance() {
            match c {
                '\\' if !escaped => {
                    escaped = true;
                    continue;
                }
                '"' if !escaped => return Ok(self.token(TokenType::StringLit(value))),
                _ => escaped = false,
            }

            value.push(c);
        }

        // end of file

        Err(LexerError::UnterminatedString(self.line, self.column))
    }

    fn scan_comment(&mut self) -> Token {
        let mut comment = String::new();

        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            } else {
                comment.push(c);
                self.advance();
            }
        }

        self.token(TokenType::Comment(comment))
    }

    fn scan_char(&mut self) -> Result<Token, LexerError> {
        if let Some(c) = self.advance() {
            if c == '\'' {
                return Err(LexerError::UnexpectedCharacter(
                    '\'',
                    self.line,
                    self.column,
                ));
            }

            if self.peek() == Some('\'') {
                self.advance();
                return Ok(self.token(TokenType::CharLit(c)));
            } else {
                return Err(LexerError::UnexpectedCharacter(c, self.line, self.column));
            }
        }

        Err(LexerError::UnexpectedCharacter(
            '\0',
            self.line,
            self.column,
        ))
    }

    fn advance(&mut self) -> Option<char> {
        if let Some(ch) = self.peek() {
            self.position += 1;
            if ch == '\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
            Some(ch)
        } else {
            None
        }
    }

    fn peek(&self) -> Option<char> {
        self.input.get(self.position).copied()
    }

    fn err_unexpected(&self, c: char) -> Option<Result<Token, LexerError>> {
        Some(Err(LexerError::UnexpectedCharacter(
            c,
            self.line,
            self.column,
        )))
    }

    fn token(&self, token_type: TokenType) -> Token {
        Token {
            token_type,
            position: self.position,
            line: self.line,
            column: self.column,
        }
    }
}


