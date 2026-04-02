use crate::tokens::Token;

pub struct Lexer {
    input: String,
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

pub enum LexerError {
    UnexpectedCharacter(char, usize, usize),
    UnterminatedString(usize, usize),
    InvalidNumber(String, usize, usize),
    InvalidOperator(String, usize, usize),
}

impl Lexer {
    pub fn new(input: String) -> Self {
        Self {
            input,
            position: 0,
            column: 1,
            line: 1,
        }
    }

    pub fn next(&mut self) -> Result<Token, LexerError> {
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
                // Identifier or reserved word
                // self.scan_literal
            } else if c.is_ascii_digit() {
                // Number
                // self.scan_number
            } else {
                self.advance();
                match c {
                    '"' | '\'' => {
                        if self.peek() == Some('/') {
                            // Comment
                            while !self.is_at_end() && self.peek() != Some('\n') {
                                self.advance();
                            }
                        } else {
                            return Ok(Token::Slash);
                        }
                    }
                    '(' => return Ok(Token::LeftParen),
                    ')' => return Ok(Token::RightParen),
                    '{' => return Ok(Token::LeftBrace),
                    '}' => return Ok(Token::RightBrace),
                    '[' => return Ok(Token::LeftBracket),
                    ']' => return Ok(Token::RightBracket),
                    '<' => return Ok(Token::LT),
                    '>' => return Ok(Token::GT),
                    '=' => {
                        if self.peek() == Some('=') {
                            self.advance();
                            return Ok(Token::EqEq);
                        } else {
                            return Ok(Token::Eq);
                        }
                    }
                    '!' => {
                        if self.peek() == Some('=') {
                            self.advance();
                            return Ok(Token::BangEq);
                        } else {
                            return Ok(Token::Bang);
                        }
                    }
                    '+' => return Ok(Token::Plus),
                    '-' => return Ok(Token::Minus),
                    '*' => return Ok(Token::Star),
                    '%' => return Ok(Token::Percent),
                    '&' => {
                        if self.peek() == Some('&') {
                            self.advance();
                            return Ok(Token::And);
                        } else {
                            return self.err_unexpected('&');
                        }
                    }
                    '|' => {
                        if self.peek() == Some('|') {
                            self.advance();
                            return Ok(Token::Or);
                        } else {
                            return Ok(Token::Pipe);
                        }
                    }
                    '.' => return Ok(Token::Dot),
                    ',' => return Ok(Token::Comma),
                    ':' => return Ok(Token::Colon),
                    ';' => return Ok(Token::Semicolon),
                }
            }
        } else {
            // EOF
        }

        todo!()
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
        self.input.chars().nth(self.position)
    }

    fn is_at_end(&self) -> bool {
        self.position == self.input.len()
    }

    fn err_unexpected(&self, c: char) -> Result<Token, LexerError> {
        Err(LexerError::UnexpectedCharacter(c, self.line, self.column))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
}
