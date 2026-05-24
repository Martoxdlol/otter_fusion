use crate::mir::*;
use std::collections::HashMap;

// ACÁ ES DONDE OCURRE LA MAGIA

/// Build function helpers.
pub struct FnBuilder {
    pub id: MirFnId,
    pub name: String,
    pub abi: Abi,

    pub params: Vec<LocalId>, // locales que son parámetros, en orden
    pub locals: HashMap<LocalId, MirLocal>, // todos los locales de la función
    pub blocks: HashMap<BlockId, MirBlock>, // todos los bloques del CFG
    pub return_type: MirType,

    pub entry: BlockId,         // bloque de entrada
    pub current_block: BlockId, // bloque donde se emiten stmts ahora

    /// Scope stack — each level is a `name → LocalId` map. Pushed on
    /// block entry, popped on block exit. Lookup walks back-to-front.
    pub scope: Vec<HashMap<String, LocalId>>,

    /// Loop stack — for each enclosing loop, (continue_target, break_target).
    /// `continue` goes to .0 (loop head), `break` to .1 (loop exit).
    pub loop_stack: Vec<(BlockId, BlockId)>,

    next_local: u32, // contador monótono de LocalId
    next_block: u32, // contador monótono de BlockId
}

impl FnBuilder {
    pub fn new(id: MirFnId, name: String, abi: Abi, return_type: MirType) -> Self {
        let mut b = Self {
            id,
            name,
            abi,
            return_type,
            params: vec![],
            locals: HashMap::new(),
            blocks: HashMap::new(),
            entry: BlockId(0),
            current_block: BlockId(0),
            scope: vec![HashMap::new()], // siempre hay un scope raíz
            loop_stack: vec![],
            next_local: 0,
            next_block: 0,
        };
        let entry = b.new_block(); // crea el bloque de entrada
        b.entry = entry;
        b.current_block = entry;
        b
    }

    pub fn new_local(&mut self, name: Option<String>, ty: MirType) -> LocalId {
        let id = LocalId(self.next_local); // id fresco
        self.next_local += 1;
        self.locals.insert(id, MirLocal { id, name, ty });
        id
    }
    pub fn new_temp(&mut self, ty: MirType) -> LocalId {
        self.new_local(None, ty) // temporal sin nombre
    }

    pub fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.next_block);
        self.next_block += 1;
        self.blocks.insert(
            id,
            MirBlock {
                id,
                stmts: vec![],
                terminator: Terminator::Unreachable, // placeholder, terminate() lo pisa
            },
        );
        id
    }

    pub fn switch_to(&mut self, b: BlockId) {
        self.current_block = b; // próximos emits van a `b`
    }

    pub fn push_stmt(&mut self, s: Stmt) {
        self.blocks
            .get_mut(&self.current_block)
            .unwrap() // current_block siempre existe
            .stmts
            .push(s);
    }

    /// Allocate a temp, emit `tmp = rv`, return Copy(tmp). Collapses the
    /// new_temp/push_stmt/Operand::Copy boilerplate.
    pub fn emit(&mut self, rv: AssignValue, ty: MirType) -> Operand {
        let tmp = self.new_temp(ty);
        self.push_stmt(Stmt::Assign(tmp, rv));
        Operand::Copy(tmp)
    }

    pub fn terminate(&mut self, t: Terminator) {
        self.blocks.get_mut(&self.current_block).unwrap().terminator = t; // pisa el Unreachable inicial
    }

    /// True iff current block has no terminator yet (still the initial Unreachable).
    pub fn is_open(&self) -> bool {
        matches!(
            self.blocks[&self.current_block].terminator,
            Terminator::Unreachable
        )
    }

    /// Set terminator only if the current block is still open.
    pub fn terminate_if_open(&mut self, t: Terminator) {
        if self.is_open() {
            self.terminate(t);
        }
    }

    pub fn push_scope(&mut self) {
        self.scope.push(HashMap::new());
    }
    pub fn pop_scope(&mut self) {
        self.scope.pop();
    }
    pub fn bind(&mut self, name: String, local: LocalId) {
        self.scope.last_mut().unwrap().insert(name, local); // bindea en el scope más interno
    }
    pub fn lookup(&self, name: &str) -> Option<LocalId> {
        self.scope.iter().rev().find_map(|s| s.get(name).copied()) // shadowing: gana el más interno
    }

    pub fn finish(self) -> MirFunction {
        MirFunction {
            id: self.id,
            name: self.name,
            abi: self.abi,
            params: self.params,
            locals: self.locals,
            blocks: self.blocks,
            entry: self.entry,
            return_type: self.return_type,
        }
    }
}
