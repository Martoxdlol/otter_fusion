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
    Extend,    // extend
    Return,    // return
    Interface, // interface
    Is,        // is
    Type,      // type
    As,        // as
    SelfRef,   // self
    Match,     // match
    Class,     // class
    Continue,  // continue
    Break,     // break

    EOF,
}
