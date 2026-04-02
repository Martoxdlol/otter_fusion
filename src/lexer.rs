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
    InvalidOperator(String, usize, usize),
}

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
                    '<' => Some(Ok(self.token(TokenType::LT))),
                    '>' => Some(Ok(self.token(TokenType::GT))),
                    '=' => {
                        if self.peek() == Some('=') {
                            self.advance();
                            return Some(Ok(self.token(TokenType::EqEq)));
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

                _ => return Err(LexerError::InvalidNumber(number, self.line, self.column)),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn next_ok(lexer: &mut Lexer) -> Token {
        match lexer.next_token() {
            Some(Ok(token)) => token,
            Some(Err(_)) => panic!("expected token, got error"),
            None => panic!("expected token, got none"),
        }
    }

    fn next_err(lexer: &mut Lexer) -> LexerError {
        match lexer.next_token() {
            Some(Err(err)) => err,
            Some(Ok(_)) => panic!("expected error, got token"),
            None => panic!("expected error, got none"),
        }
    }

    fn assert_eof(token: Token) {
        assert!(matches!(token.token_type, TokenType::EOF));
    }

    #[test]
    fn scan_number_valid_int() {
        let mut lexer = Lexer::new("123");
        let token = next_ok(&mut lexer);

        assert_eq!(token.token_type, TokenType::Int("123".to_string()));
    }

    #[test]
    fn scan_number_valid_float() {
        let mut lexer = Lexer::new("123.02");
        let token = next_ok(&mut lexer);
        assert_eq!(token.token_type, TokenType::Float("123.02".to_string()));
    }
    #[test]
    fn scan_number_invalid_float_non_digit_after_dot() {
        let mut lexer = Lexer::new("123.abc");
        next_err(&mut lexer);
    }

    #[test]
    fn scan_number_invalid_float_none_after_dot() {
        let mut lexer = Lexer::new("123.");
        next_err(&mut lexer);
    }

    #[test]
    fn scan_number_invalid_float_multiple_point() {
        let mut lexer = Lexer::new("123.02.5");
        next_err(&mut lexer);
    }

    #[test]
    fn empty_input_returns_eof() {
        let mut lexer = Lexer::new("");
        let token = next_ok(&mut lexer);
        assert_eof(token);
    }

    #[test]
    fn whitespace_only_returns_eof() {
        let mut lexer = Lexer::new("   \n\t  ");
        let token = next_ok(&mut lexer);
        assert_eof(token);
    }

    #[test]
    fn identifier_is_scanned() {
        let mut lexer = Lexer::new("hello");
        let token = next_ok(&mut lexer);

        match token.token_type {
            TokenType::Identifier(value) => assert_eq!(value, "hello"),
            _ => panic!("expected identifier"),
        }
    }

    #[test]
    fn identifier_with_underscore_dollar_and_digits_is_scanned() {
        let mut lexer = Lexer::new("_value$123");
        let token = next_ok(&mut lexer);

        match token.token_type {
            TokenType::Identifier(value) => assert_eq!(value, "_value$123"),
            _ => panic!("expected identifier"),
        }
    }

    #[test]
    fn keywords_are_recognized() {
        let mut lexer =
            Lexer::new("struct function for while null true false var extend return interface");

        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Struct));
        assert!(matches!(
            next_ok(&mut lexer).token_type,
            TokenType::Function
        ));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::For));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::While));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Null));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::True));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::False));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Var));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Extend));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Return));
        assert!(matches!(
            next_ok(&mut lexer).token_type,
            TokenType::Interface
        ));
        assert_eof(next_ok(&mut lexer));
    }

    #[test]
    fn non_keyword_literal_becomes_identifier() {
        let mut lexer = Lexer::new("structure");
        let token = next_ok(&mut lexer);

        match token.token_type {
            TokenType::Identifier(value) => assert_eq!(value, "structure"),
            _ => panic!("expected identifier"),
        }
    }

    #[test]
    fn punctuation_tokens_are_recognized() {
        let mut lexer = Lexer::new("(){}[].,:;");

        assert!(matches!(
            next_ok(&mut lexer).token_type,
            TokenType::LeftParen
        ));
        assert!(matches!(
            next_ok(&mut lexer).token_type,
            TokenType::RightParen
        ));
        assert!(matches!(
            next_ok(&mut lexer).token_type,
            TokenType::LeftBrace
        ));
        assert!(matches!(
            next_ok(&mut lexer).token_type,
            TokenType::RightBrace
        ));
        assert!(matches!(
            next_ok(&mut lexer).token_type,
            TokenType::LeftBracket
        ));
        assert!(matches!(
            next_ok(&mut lexer).token_type,
            TokenType::RightBracket
        ));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Dot));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Comma));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Colon));
        assert!(matches!(
            next_ok(&mut lexer).token_type,
            TokenType::Semicolon
        ));
        assert_eof(next_ok(&mut lexer));
    }

    #[test]
    fn arithmetic_and_comparison_tokens_are_recognized() {
        let mut lexer = Lexer::new("+-*/%<>");

        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Plus));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Minus));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Star));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Slash));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Percent));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::LT));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::GT));
        assert_eof(next_ok(&mut lexer));
    }

    #[test]
    fn equality_and_negation_tokens_are_recognized() {
        let mut lexer = Lexer::new("= == ! !=");

        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Eq));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::EqEq));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Bang));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::BangEq));
        assert_eof(next_ok(&mut lexer));
    }

    #[test]
    fn pipe_and_or_tokens_are_recognized() {
        let mut lexer = Lexer::new("| ||");

        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Pipe));
        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Or));
        assert_eof(next_ok(&mut lexer));
    }

    #[test]
    fn double_ampersand_is_and_token() {
        let mut lexer = Lexer::new("&&");
        let token = next_ok(&mut lexer);

        assert!(matches!(token.token_type, TokenType::And));
    }

    #[test]
    fn single_ampersand_is_unexpected_character() {
        let mut lexer = Lexer::new("&");

        match next_err(&mut lexer) {
            LexerError::UnexpectedCharacter(c, _, _) => assert_eq!(c, '&'),
            _ => panic!("expected unexpected character error"),
        }
    }

    #[test]
    fn unexpected_character_returns_error() {
        let mut lexer = Lexer::new("@");

        match next_err(&mut lexer) {
            LexerError::UnexpectedCharacter(c, _, _) => assert_eq!(c, '@'),
            _ => panic!("expected unexpected character error"),
        }
    }

    #[test]
    fn slash_starts_comment_when_followed_by_slash() {
        let mut lexer = Lexer::new("// comment\nvar");

        let _ = next_ok(&mut lexer);

        let token = next_ok(&mut lexer);
        assert!(matches!(token.token_type, TokenType::Var));
    }

    #[test]
    fn comment_at_end_of_input_results_in_eof() {
        let mut lexer = Lexer::new("// comment");
        let _ = next_ok(&mut lexer);
        let token = next_ok(&mut lexer);

        assert_eof(token);
    }

    #[test]
    fn slash_without_second_slash_is_slash_token() {
        let mut lexer = Lexer::new("/");
        let token = next_ok(&mut lexer);

        assert!(matches!(token.token_type, TokenType::Slash));
    }

    #[test]
    fn string_literal_is_scanned() {
        let mut lexer = Lexer::new("\"hello world\"");
        let token = next_ok(&mut lexer);

        match token.token_type {
            TokenType::StringLit(value) => assert_eq!(value, "hello world"),
            _ => panic!("expected string literal"),
        }
    }

    #[test]
    fn escaped_quote_is_kept_inside_string() {
        let mut lexer = Lexer::new("\"hello \\\"world\\\"\"");
        let token = next_ok(&mut lexer);

        match token.token_type {
            TokenType::StringLit(value) => assert_eq!(value, "hello \"world\""),
            _ => panic!("expected string literal"),
        }
    }

    #[test]
    fn escaped_backslash_is_kept_inside_string() {
        let mut lexer = Lexer::new("\"a\\\\b\"");
        let token = next_ok(&mut lexer);

        match token.token_type {
            TokenType::StringLit(value) => assert_eq!(value, "a\\b"),
            _ => panic!("expected string literal"),
        }
    }

    #[test]
    fn unterminated_string_returns_error() {
        let mut lexer = Lexer::new("\"unterminated");

        match next_err(&mut lexer) {
            LexerError::UnterminatedString(_, _) => {}
            _ => panic!("expected unterminated string error"),
        }
    }

    #[test]
    fn token_position_line_and_column_are_updated() {
        let mut lexer = Lexer::new(" \nfoo");
        let token = next_ok(&mut lexer);

        match token.token_type {
            TokenType::Identifier(value) => assert_eq!(value, "foo"),
            _ => panic!("expected identifier"),
        }

        assert_eq!(token.line, 2);
        assert_eq!(token.column, 4);
        assert_eq!(token.position, 5);
    }

    #[test]
    fn comment_then_identifier_on_later_line_has_correct_line() {
        let mut lexer = Lexer::new("// first line\nbar");

        let comment = next_ok(&mut lexer);

        match comment.token_type {
            TokenType::Comment(value) => assert_eq!(value, " first line"),
            _ => panic!("expected comment"),
        }

        let token = next_ok(&mut lexer);

        match token.token_type {
            TokenType::Identifier(value) => assert_eq!(value, "bar"),
            _ => panic!("expected identifier"),
        }

        assert_eq!(token.line, 2);
    }

    #[test]
    fn iterator_trait_next_can_be_used() {
        let mut lexer = Lexer::new("var");

        let token = match Iterator::next(&mut lexer) {
            Some(Ok(token)) => token,
            Some(Err(_)) => panic!("expected token, got error"),
            None => panic!("expected token, got none"),
        };

        assert!(matches!(token.token_type, TokenType::Var));
    }

    #[test]
    fn mixed_sequence_of_tokens_is_scanned_in_order() {
        let mut lexer = Lexer::new("var foo = \"bar\";");

        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Var));

        match next_ok(&mut lexer).token_type {
            TokenType::Identifier(value) => assert_eq!(value, "foo"),
            _ => panic!("expected identifier"),
        }

        assert!(matches!(next_ok(&mut lexer).token_type, TokenType::Eq));

        match next_ok(&mut lexer).token_type {
            TokenType::StringLit(value) => assert_eq!(value, "bar"),
            _ => panic!("expected string literal"),
        }

        assert!(matches!(
            next_ok(&mut lexer).token_type,
            TokenType::Semicolon
        ));
        assert_eof(next_ok(&mut lexer));
    }
}
