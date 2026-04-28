use std::collections::HashMap;

use crate::hir::PrimitiveType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MirTypeId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MirFnId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub u32);

#[derive(Debug, Clone)]
pub struct MirProgram {
    // Types used in params, variables, etc...
    pub types: HashMap<MirTypeId, MirTypeDef>,
    // functions with generics resolved yey!!!
    pub functions: HashMap<MirFnId, MirFunction>,
    /// Resolve interfaces to correct type
    /// (struct, interface) pairs for dynamic dispatch.
    pub vtables: HashMap<(MirTypeId, MirTypeId), VTable>,
    // main
    pub entry: MirFnId,
}

/// Types definitions (can be referenced on other parts)
#[derive(Debug, Clone)]
pub enum MirTypeDef {
    /// Any struct allocated via `AllocStruct` lives on the GC heap and has
    /// this layout:
    /// `[gc header | type id | field 1 | field 2 | ...]`
    /// Pointer points to field 1 (so type id is at offset -1, header is `at offset -2).
    ///
    /// Structs coming from C doesn't have header and can be distinguished looking the address.
    /// Extern structs allocated by us do have a header and the GC will check it to know if it has
    /// to manage it or not.
    Struct {
        name: String,
        fields: Vec<MirField>,
        layout: Layout,
        /// If foreign, we must be sure to make it C compatible
        /// Restriction 1: cannot implement interfaces (sorry)
        /// Restriction 2: no casting coercion or weird stuff
        /// Restriction 3: cannot be part of generics
        /// Restriction 4: shouldn't probably have managed fields (maybe with pin/unpin can be supported)
        kind: StructKind,
    },
    /// tag will be 2 bytes (u16)
    Union {
        // variants of the union
        variants: Vec<UnionVariant>,
        /// In practice is always [type, ptr] (align usize)
        /// But if they are primitives like (i8, f8) we can use smaller align
        layout: Layout,
    },
    /// Don't care for now
    Closure {
        /// It is like a struct, with access to variables of when it was created
        env_fields: Vec<MirField>,
        function: MirFnId,
        layout: Layout,
    },
}

/// Struct field
#[derive(Debug, Clone)]
pub struct MirField {
    pub name: String,
    pub ty: MirType,
}

/// Union variant
#[derive(Debug, Clone)]
pub struct UnionVariant {
    pub tag: u16,
    pub ty: MirType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructKind {
    Managed, // GC-allocated (by us)
    Extern,  // FFI / extern struct: C-ABI layout, FFI-safe fields. May be
             // allocated by us (GC heap, header-bearing) or received from
             // C (headerless); origin is checked at runtime by address.
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// size is total length
/// align is to make all elements take at least a multiple of align (for cache efficiency things)
pub struct Layout {
    pub size: u32,
    pub align: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MirType {
    /// Numbers, chars, bools
    Primitive(PrimitiveType),
    /// GC-managed reference: structs, lists, maps, strings, closures.
    ManagedRef(MirTypeId),
    /// Raw pointer — extern boundary only.
    Pointer(Box<MirType>),
    /// Union of types
    Union(MirTypeId),
    /// Bare extern function pointer
    FnPtr(Vec<MirType>, Box<MirType>),
    /// Managed closure with environment. I don't care for now
    Closure(MirTypeId),
    /// Nullable managed reference - null-pointer optimization for `T | null`.
    NullableRef(MirTypeId),
}

/// In code I can now a variable is a interface (mir type id)
/// I can also know the type id
#[derive(Debug, Clone)]
pub struct VTable {
    pub struct_ty: MirTypeId,
    pub interface_ty: MirTypeId,
    /// Interface slot index → concrete monomorphized function.
    pub slots: Vec<MirFnId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Abi {
    /// Whatever the default is for our language
    Otter,
    /// C compatible ABI for FFI functions
    Extern,
}

#[derive(Debug, Clone)]
pub struct MirFunction {
    /// Unique identifier for this function.
    pub id: MirFnId,
    /// name
    pub name: String,
    /// Extern or not
    pub abi: Abi,
    /// What local variables are actually parameters (in order).
    pub params: Vec<LocalId>,
    /// Local variables
    pub locals: HashMap<LocalId, MirLocal>,
    /// Code blocks
    pub blocks: HashMap<BlockId, MirBlock>,
    /// Initial block
    pub entry: BlockId,
    /// Return type
    pub return_type: MirType,
}

/// Local variable / fun param
#[derive(Debug, Clone)]
pub struct MirLocal {
    pub id: LocalId,
    pub name: Option<String>, // user-visible name, if any
    pub ty: MirType,
}

/// Basic code block
#[derive(Debug, Clone)]
pub struct MirBlock {
    pub id: BlockId,
    pub stmts: Vec<Stmt>,
    pub terminator: Terminator,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Assign(LocalId, AssignValue),
}

/// The value at the right
#[derive(Debug, Clone)]
pub enum AssignValue {
    /// Just copy or move another local variable or constant.
    Use(Operand),
    /// Binary operation (+ - * / % && || == != < <= > >=)
    Bin(BinOp, Operand, Operand),
    /// Unary operation (-, !)
    Un(UnOp, Operand),
    /// Fn call
    Call(Callee, Vec<Operand>),
    /// Struct
    AllocStruct(MirTypeId, Vec<Operand>),
    /// List is primitive (because of syntax)
    AllocList(MirType, Vec<Operand>),
    /// Map is primitive (because of syntax)
    AllocMap(MirType, MirType, Vec<(Operand, Operand)>),
    /// I don't care
    AllocClosure(MirFnId, Vec<Operand>),
    /// Access struct field or other things with a layout
    Field(Operand, u32),
    /// Construct a union value
    /// Union type, tag (from the struct we are constructing), payload
    /// Sits in stack (the struct with the type and gc data do live in heap)
    UnionConstruct(MirTypeId, u32, Operand), // Unions with tag
    /// read the discriminant (extract the tag) of a union value
    UnionTag(Operand),
    /// Unchecked payload extraction — must be guarded by a UnionTag check.
    UnionPayload(Operand, MirTypeId),
}

#[derive(Debug, Clone)]
pub enum Operand {
    /// Just copy the value (primitive or pointer, not struct data)
    Copy(LocalId),
    /// The same as copy but it is a way of saying we don't need the original value anymore
    Move(LocalId),
    /// A constant value
    Const(MirConst),
}

#[derive(Debug, Clone)]
pub enum MirConst {
    Int(i64, PrimitiveType),
    Float(f64, PrimitiveType),
    Bool(bool),
    Char(char),
    String(String),
    Null,
    /// Address of a static/extern function (e.g. when used as a value).
    Fn(MirFnId),
}

#[derive(Debug, Clone)]
pub enum Callee {
    /// Monomorphized direct call.
    Static(MirFnId),
    /// Indirect call through a closure or function-pointer operand.
    Indirect(Operand),
    /// Interface dispatch: receiver, interface type, slot index.
    Virtual(Operand, MirTypeId, u32),
}

#[derive(Debug, Clone)]
pub enum Terminator {
    Goto(BlockId),
    CondBr(Operand, BlockId, BlockId),
    /// Switch on integer value (used for union discriminants and `is` chains).
    Switch {
        scrutinee: Operand,
        arms: Vec<(u64, BlockId)>,
        default: BlockId,
    },
    Return(Option<Operand>),
    Trap(TrapReason),
    Unreachable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrapReason {
    /// `as T` didn't match
    AsMismatch,
    /// Try to access null pointer from ffi
    NullDeref,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}
