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
    Type(TypeId, String),      // (resolved id, local name/alias)
    Function(FnId, String),    // (resolved id, local name/alias)
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
    Alias(TypeId, Vec<ResolvedType>), // recursive type alias reference
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

/// Methods available only for a specific type argument combination
#[derive(Debug, Clone)]
pub struct SpecializedExtend {
    pub type_args: Vec<ResolvedType>,
    pub methods: Vec<FnId>,
}

/// Struct with all extend methods merged in
#[derive(Debug, Clone)]
pub struct HirStruct {
    pub id: TypeId,
    pub module: ModuleId,
    pub name: String,
    pub type_params: Vec<TypeParamId>,
    pub fields: Vec<HirField>,
    pub methods: Vec<FnId>,      // generic methods (includes generic extend blocks)
    pub implements: Vec<TypeId>, // generic interface IDs
    pub specialized_methods: Vec<SpecializedExtend>,
    pub specialized_implements: Vec<(Vec<ResolvedType>, TypeId)>, // (type_args, iface_id)
}

#[derive(Debug, Clone)]
pub struct HirInterface {
    pub id: TypeId,
    pub module: ModuleId,
    pub name: String,
    pub type_params: Vec<TypeParamId>,
    pub fields: Vec<HirField>,
    pub methods: Vec<FnId>,   // abstract + default methods
    pub extends: Vec<TypeId>, // parent interfaces
}

#[derive(Debug, Clone)]
pub struct HirField {
    pub name: String,
    pub ty: ResolvedType,
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
