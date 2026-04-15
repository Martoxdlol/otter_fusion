use std::fmt;

#[derive(Debug, Clone)]
pub struct ValidationError {
    pub kind: ErrorKind,
    pub module: String,
    pub context: Option<String>,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[module '{}']", self.module)?;
        if let Some(ctx) = &self.context {
            write!(f, " {ctx}")?;
        }
        write!(f, ": {}", self.kind)
    }
}

#[derive(Debug, Clone)]
pub enum ErrorKind {
    // Name resolution
    UndefinedType(String),
    UndefinedFunction(String),
    UndefinedVariable(String),
    UndefinedModule(String),
    UndefinedMember { ty: String, member: String },
    DuplicateType { name: String },
    DuplicateFunction { name: String },
    DuplicateField { name: String, owner: String },
    DuplicateVariable(String),

    // Type checking
    TypeMismatch { expected: String, found: String },
    BinaryOpTypeMismatch { op: String, left: String, right: String },
    UnaryOpTypeMismatch { op: String, operand: String },
    NotCallable(String),
    WrongArgCount { expected: usize, found: usize },
    WrongTypeArgCount { expected: usize, found: usize },
    MissingField { struct_name: String, field: String },
    ExtraField { struct_name: String, field: String },
    ConditionNotBool(String),
    BreakOutsideLoop,
    ContinueOutsideLoop,

    // Type alias
    CyclicTypeAlias(String),

    // Interface
    MissingInterfaceField { iface: String, field: String },
    MissingInterfaceMethod { iface: String, method: String },
    MethodSignatureMismatch { iface: String, method: String, detail: String },

    // Extend
    ExtendTargetNotStruct(String),

    // Import
    ImportSymbolNotFound { module: String, symbol: String },

    // Generics
    TypeParamBoundNotInterface(String),

    // Literals
    InvalidIntLiteral(String),
    InvalidFloatLiteral(String),

    // Var decl
    CannotInferType(String),

    // Return
    ReturnTypeMismatch { expected: String, found: String },

    // For loop
    NotIterable(String),
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::UndefinedType(name) => write!(f, "undefined type '{name}'"),
            ErrorKind::UndefinedFunction(name) => write!(f, "undefined function '{name}'"),
            ErrorKind::UndefinedVariable(name) => write!(f, "undefined variable '{name}'"),
            ErrorKind::UndefinedModule(name) => write!(f, "undefined module '{name}'"),
            ErrorKind::UndefinedMember { ty, member } => {
                write!(f, "type '{ty}' has no member '{member}'")
            }
            ErrorKind::DuplicateType { name } => write!(f, "duplicate type '{name}'"),
            ErrorKind::DuplicateFunction { name } => write!(f, "duplicate function '{name}'"),
            ErrorKind::DuplicateField { name, owner } => {
                write!(f, "duplicate field '{name}' on '{owner}'")
            }
            ErrorKind::DuplicateVariable(name) => write!(f, "duplicate variable '{name}'"),
            ErrorKind::TypeMismatch { expected, found } => {
                write!(f, "type mismatch: expected '{expected}', found '{found}'")
            }
            ErrorKind::BinaryOpTypeMismatch { op, left, right } => {
                write!(f, "cannot apply '{op}' to '{left}' and '{right}'")
            }
            ErrorKind::UnaryOpTypeMismatch { op, operand } => {
                write!(f, "cannot apply '{op}' to '{operand}'")
            }
            ErrorKind::NotCallable(ty) => write!(f, "type '{ty}' is not callable"),
            ErrorKind::WrongArgCount { expected, found } => {
                write!(f, "expected {expected} arguments, found {found}")
            }
            ErrorKind::WrongTypeArgCount { expected, found } => {
                write!(f, "expected {expected} type arguments, found {found}")
            }
            ErrorKind::MissingField { struct_name, field } => {
                write!(f, "missing field '{field}' in struct '{struct_name}'")
            }
            ErrorKind::ExtraField { struct_name, field } => {
                write!(f, "unknown field '{field}' in struct '{struct_name}'")
            }
            ErrorKind::ConditionNotBool(ty) => {
                write!(f, "condition must be bool, found '{ty}'")
            }
            ErrorKind::BreakOutsideLoop => write!(f, "'break' outside of loop"),
            ErrorKind::ContinueOutsideLoop => write!(f, "'continue' outside of loop"),
            ErrorKind::CyclicTypeAlias(name) => {
                write!(f, "cyclic type alias '{name}'")
            }
            ErrorKind::MissingInterfaceField { iface, field } => {
                write!(f, "missing field '{field}' required by interface '{iface}'")
            }
            ErrorKind::MissingInterfaceMethod { iface, method } => {
                write!(f, "missing method '{method}' required by interface '{iface}'")
            }
            ErrorKind::MethodSignatureMismatch {
                iface,
                method,
                detail,
            } => {
                write!(
                    f,
                    "method '{method}' signature doesn't match interface '{iface}': {detail}"
                )
            }
            ErrorKind::ExtendTargetNotStruct(name) => {
                write!(f, "extend target '{name}' is not a struct")
            }
            ErrorKind::ImportSymbolNotFound { module, symbol } => {
                write!(f, "symbol '{symbol}' not found in module '{module}'")
            }
            ErrorKind::TypeParamBoundNotInterface(name) => {
                write!(f, "type parameter bound '{name}' is not an interface")
            }
            ErrorKind::InvalidIntLiteral(s) => write!(f, "invalid integer literal '{s}'"),
            ErrorKind::InvalidFloatLiteral(s) => write!(f, "invalid float literal '{s}'"),
            ErrorKind::CannotInferType(name) => {
                write!(f, "cannot infer type for '{name}': no type annotation or initializer")
            }
            ErrorKind::ReturnTypeMismatch { expected, found } => {
                write!(f, "return type mismatch: expected '{expected}', found '{found}'")
            }
            ErrorKind::NotIterable(ty) => write!(f, "type '{ty}' is not iterable"),
        }
    }
}
