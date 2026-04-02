pub struct DebugData {
    pub dubug_stack: Vec<DebugPhase>,
}

pub enum DebugPhase {
    Lexer(LexerDebugData),
    Parser(),
}

pub struct LexerDebugData {
    pub position: usize,
    pub column: usize,
    pub line: usize,
}

pub struct ParserDebugData {
    pub consumed_tokens: Vec<Token>,
}
