#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    TypeAlias(TypeAliasDecl),
    Interface(InterfaceDecl),
    Struct(StructDecl),
    Function(FunctionDecl),
    Extend(ExtendDecl),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeAliasDecl {
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GenericParam {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    Primitive(PrimitiveType),
    Union(Vec<TypeExpr>),
    Named(String, Vec<TypeExpr>),
    Function(Vec<TypeExpr>, Box<TypeExpr>), // (param_types) -> return_type
}

#[derive(Debug, Clone, PartialEq)]
pub enum PrimitiveType {
    Int8,
    Int16,
    Int32,
    Int64,
    Uint8,
    Uint16,
    Uint32,
    Uint64,
    Float32,
    Float64,
    Bool,
    String,
    Char,
    Null,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceDecl {
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub fields: Vec<FieldDecl>,
    pub methods: Vec<FunctionDecl>,
    pub implements: Vec<TypeExpr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl {
    pub name: String,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub returns: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDecl {
    pub name: String,
    pub has_self_param: bool,
    pub generics: Vec<GenericParam>,
    pub return_type: Option<TypeExpr>,
    pub params: Vec<ParamDecl>,
    pub body: Option<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamDecl {
    pub name: String,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    VarDecl(String, Option<TypeExpr>, Option<Expr>), // var x: int = 5; / var x = 5;
    Return(Option<Expr>),                            // return 5;
    Expr(Expr),                                      // x + 5;
    While(Expr, Block),                              // while x < 10 { ... }
    For(String, Expr, Block),                        // for (item in items) { ... }
    Break,                                           // break
    Continue,                                        // continue
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Literal),                               // 5, "hello", true
    Variable(String),                               // x
    If(Box<Expr>, Box<Block>, Option<Box<Block>>),  // if cond { then } else { else }
    Call(Box<Expr>, Vec<TypeExpr>, Vec<Expr>),      // func<T>(args)
    LiteralMap(Vec<(Expr, Expr)>),                  // { "x": 1, "y": 2 }
    LiteralList(Vec<Expr>),                         // [1, 2, 3]
    StructInit(TypeExpr, Vec<(String, Expr)>),      // Point { x: 1, y: 2 }
    As(Box<Expr>, TypeExpr),                        // x as int32
    Is(Box<Expr>, TypeExpr),                        // x is int32
    Member(Box<Expr>, String),                      // point.x
    BinaryOp(Box<Expr>, BinaryOperator, Box<Expr>), // x + y
    UnaryOp(UnaryOperator, Box<Expr>),              // -x
    FunctionLiteral(Vec<GenericParam>, Vec<ParamDecl>, TypeExpr, Box<Block>), // (x: int): int { ... }
    Block(Box<Block>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Int(String),    // 123
    Float(String),  // 123.0, 3.14
    String(String), // "hello"
    Char(char),     // 'a'
    Bool(bool),     // true, false
    Null,           // null
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinaryOperator {
    Add, // +
    Sub, // -
    Mul, // *
    Div, // /
    Mod, // %
    And, // &&
    Or,  // ||
    Eq,  // ==
    Neq, // !=
    Lt,  // <
    Le,  // <=
    Gt,  // >
    Ge,  // >=
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOperator {
    Neg, // -
    Not, // !
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDecl {
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub fields: Vec<FieldDecl>,
    pub methods: Vec<FunctionDecl>,
    pub implements: Vec<TypeExpr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtendDecl {
    pub target: TypeExpr,
    pub generic_params: Vec<GenericParam>,
    pub implements: Vec<TypeExpr>,
    pub methods: Vec<FunctionDecl>,
}
