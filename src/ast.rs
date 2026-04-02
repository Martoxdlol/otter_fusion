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
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl {
    pub name: String,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDecl {
    pub name: String,
    pub has_self_param: bool,
    pub generics: Vec<GenericParam>,
    pub return_type: TypeExpr,
    pub params: Vec<ParamDecl>,
    pub body: Option<Vec<Statement>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamDecl {
    pub name: String,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    VarDecl(String, TypeExpr, Option<Expr>), // var x: int = 5;
    Return(Option<Expr>),                    // return 5;
    Expr(Expr),                              // x + 5;
    For(String, Expr, Vec<Statement>),       // for i in 0..10 { ... }
    While(Expr, Vec<Statement>),             // while x < 10 { ... }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Literal),                               // 5, "hello", true
    Variable(String),                               // x
    If(Box<Expr>, Box<Expr>, Option<Box<Expr>>),    // if cond { then } else { else }
    Call(Box<Expr>, Vec<TypeExpr>, Vec<Expr>),      // func(args)
    LiteralMap(Vec<(String, Expr)>),                // { x: 1, y: 2 }
    LiteralList(Vec<Expr>),                         // [1, 2, 3]
    StructInit(String, Vec<(String, Expr)>),        // Point { x: 1, y: 2 }
    As(Box<Expr>, TypeExpr),                        // x as int32
    Is(Box<Expr>, TypeExpr),                        // x is int32
    Member(Box<Expr>, String),                      // point.x
    BinaryOp(Box<Expr>, BinaryOperator, Box<Expr>), // x + y
    UnaryOp(UnaryOperator, Box<Expr>),              // -x
    FunctionLiteral(Vec<GenericParam>, Vec<ParamDecl>, TypeExpr, Vec<Statement>), // (x: int): int { ... }
    FunctionDecl(
        String,
        Vec<GenericParam>,
        Vec<ParamDecl>,
        TypeExpr,
        Vec<Statement>,
    ), // function foo(x: int): int { ... }
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
    Gt,  // >
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
    pub implements: Vec<String>, // List of interfaces this struct implements
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtendDecl {
    pub struct_name: String,
    pub generic_params: Vec<GenericParam>,
    pub generics: Vec<TypeExpr>,
    pub methods: Vec<FunctionDecl>,
}
