pub enum Token {
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
    Float(f64),         // 123.0, 3.14
    Int(i64),           // 123

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

    EOF,
}
