use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FnId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeParamId(pub u32);

#[derive(Debug, Clone)]
pub struct TypeParamInfo {
    pub id: TypeParamId,
    pub name: String,
    pub bounds: Vec<TypeId>, // interfaces this param must implement
}

#[derive(Debug, Clone)]
pub struct HirModule {
    pub id: ModuleId,
    pub name: String,
    pub structs: Vec<TypeId>,
    pub interfaces: Vec<TypeId>,
    pub functions: Vec<FnId>,
    pub imports: Vec<HirImport>,
}

#[derive(Debug, Clone)]
pub enum HirImport {
    Glob(ModuleId),
    Named(ModuleId, Vec<HirImportSymbol>),
}

#[derive(Debug, Clone)]
pub enum HirImportSymbol {
    Type(TypeId, String),   // (resolved id, local name/alias)
    Function(FnId, String), // (resolved id, local name/alias)
    Alias {
        source: ModuleId,
        original: String, // alias name in the source module
        local: String,    // local name in the importing module
    },
}

#[derive(Debug, Clone, Default)]
pub struct Hir {
    pub modules: HashMap<ModuleId, HirModule>,
    pub structs: HashMap<TypeId, HirStruct>,
    pub interfaces: HashMap<TypeId, HirInterface>,
    pub functions: HashMap<FnId, HirFunction>,
    pub type_params: HashMap<TypeParamId, TypeParamInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedType {
    Primitive(PrimitiveType),
    Struct(TypeId, Vec<ResolvedType>), // Foo<i32, str>
    Interface(TypeId, Vec<ResolvedType>),
    Union(Vec<ResolvedType>),
    Function(Vec<ResolvedType>, Box<ResolvedType>),
    TypeParam(TypeParamId), // unresolved generic, resolved later
    Null,
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
}

/// Struct with all extend methods merged in
#[derive(Debug, Clone)]
pub struct HirStruct {
    pub id: TypeId,
    pub module: ModuleId,
    pub name: String,
    /// `extern struct` — value-typed, C-ABI-compatible, no GC header. See
    /// the foreign-struct rules enforced in the validator.
    pub is_extern: bool,
    pub type_params: Vec<TypeParamId>,
    pub fields: Vec<HirField>,
    /// Methods declared inline on the struct.
    pub methods: Vec<FnId>,
    /// Methods contributed by extend blocks. Each entry's `Vec<ResolvedType>`
    /// is the extend's target type-argument list — phase 5 unifies these
    /// with the instance's type args to determine applicability. The
    /// "universal" extend `extend<T> Foo<T>` shows up here too with args
    /// like `[TypeParam(T_id)]`.
    pub specialised_methods: Vec<(Vec<ResolvedType>, FnId)>,
    /// Implemented interfaces with their type arguments.
    pub implements: Vec<(TypeId, Vec<ResolvedType>)>,
}

#[derive(Debug, Clone)]
pub struct HirInterface {
    pub id: TypeId,
    pub module: ModuleId,
    pub name: String,
    pub type_params: Vec<TypeParamId>,
    pub fields: Vec<HirField>,
    pub methods: Vec<FnId>, // abstract + default methods
    /// Parent interfaces with their type arguments.
    pub extends: Vec<(TypeId, Vec<ResolvedType>)>,
}

#[derive(Debug, Clone)]
pub struct HirField {
    pub name: String,
    pub ty: ResolvedType,
    pub is_pointer: bool,
}

#[derive(Debug, Clone)]
pub struct HirFunction {
    pub id: FnId,
    pub module: ModuleId,
    pub name: String,
    pub owner: Option<TypeId>, // None for free functions
    pub has_self: bool,
    pub type_params: Vec<TypeParamId>,
    pub params: Vec<HirParam>,
    pub return_type: ResolvedType,
    pub body: Option<HirBlock>, // None for abstract interface methods
}

#[derive(Debug, Clone)]
pub struct HirParam {
    pub name: String,
    pub ty: ResolvedType,
    pub is_pointer: bool,
}

#[derive(Debug, Clone)]
pub struct HirBlock {
    pub statements: Vec<HirStatement>,
    pub returns: Option<TypedExpr>, // implicit return expression
}

#[derive(Debug, Clone)]
pub enum HirStatement {
    VarDecl(String, ResolvedType, Option<TypedExpr>), // type always resolved (inferred or explicit)
    Return(Option<TypedExpr>),
    Expr(TypedExpr),
    While(TypedExpr, HirBlock),
    For(String, TypedExpr, HirBlock),
    Break,
    Continue,
}

#[derive(Debug, Clone)]
pub struct TypedExpr {
    pub kind: ExprKind,
    pub ty: ResolvedType,
}

#[derive(Debug, Clone)]
pub struct HirCapture {
    pub name: String,
    pub ty: ResolvedType,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Literal(HirLiteral),
    Variable(String),
    If(Box<TypedExpr>, Box<HirBlock>, Option<Box<HirBlock>>),
    Call(Box<TypedExpr>, Vec<ResolvedType>, Vec<TypedExpr>), // callee, type args, args
    LiteralMap(Vec<(TypedExpr, TypedExpr)>),
    LiteralList(Vec<TypedExpr>),
    StructInit(TypeId, Vec<ResolvedType>, Vec<(String, TypedExpr)>),
    As(Box<TypedExpr>, ResolvedType),
    Is(Box<TypedExpr>, ResolvedType),
    Member(Box<TypedExpr>, String),
    BinaryOp(Box<TypedExpr>, BinaryOperator, Box<TypedExpr>),
    UnaryOp(UnaryOperator, Box<TypedExpr>),
    FunctionLiteral(
        Vec<TypeParamId>,
        Vec<HirParam>,
        Vec<HirCapture>,
        Box<HirBlock>,
    ),
    Block(Box<HirBlock>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinaryOperator {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    Eq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOperator {
    Neg,
    Not,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirLiteral {
    Int(i64),   // parsed from string, validated
    Float(f64), // parsed from string, validated
    String(String),
    Char(char),
    Bool(bool),
    Null,
}
