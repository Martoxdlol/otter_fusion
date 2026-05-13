use crate::{
    hir::{
        BinaryOperator, ExprKind, FnId, HirBlock, HirLiteral, PrimitiveType, ResolvedType, TypeId,
        TypedExpr, UnaryOperator,
    },
    lower::{Lower, builder::FnBuilder, subst::Subst},
    mir::{
        AssignValue, BinOp, Callee, MirConst, MirType, MirTypeDef, Operand, Stmt, Terminator, UnOp,
    },
};
use std::collections::HashMap;

impl Lower {
    pub fn lower_expr(&mut self, expr: &TypedExpr, subst: &Subst, b: &mut FnBuilder) -> Operand {
        match &expr.kind {
            ExprKind::Literal(lit) => self.lower_literal(lit, &expr.ty),
            ExprKind::Variable(name) => self.lower_variable(name, b),
            ExprKind::BinaryOp(l, op, r) => self.lower_binop(l, op, r, &expr.ty, subst, b),
            ExprKind::UnaryOp(op, e) => self.lower_unop(op, e, &expr.ty, subst, b),
            ExprKind::Call(callee, ta, args) => {
                self.lower_call(callee, ta, args, &expr.ty, subst, b)
            }
            ExprKind::StructInit(id, ta, fs) => self.lower_struct_init(*id, ta, fs, subst, b),
            ExprKind::Member(recv, field) => self.lower_member(recv, field, &expr.ty, subst, b),
            ExprKind::As(e, target) => self.lower_as(e, target, &expr.ty, subst, b),
            ExprKind::Is(_, _) => unimplemented!("Phase 2: is"),
            ExprKind::If(c, t, e) => self.lower_if(c, t, e.as_deref(), &expr.ty, subst, b),
            ExprKind::Block(blk) => self.lower_block_expr(blk, &expr.ty, subst, b),
            ExprKind::LiteralList(items) => self.lower_list_lit(items, &expr.ty, subst, b),
            ExprKind::LiteralMap(pairs) => self.lower_map_lit(pairs, &expr.ty, subst, b),
            ExprKind::FunctionLiteral(..) => unimplemented!("Phase 3: closures"),
        }
    }

    fn lower_literal(&self, lit: &HirLiteral, ty: &ResolvedType) -> Operand {
        match lit {
            HirLiteral::Int(n) => Operand::Const(MirConst::Int(*n, prim_of(ty))),
            HirLiteral::Float(f) => Operand::Const(MirConst::Float(*f, prim_of(ty))),
            HirLiteral::Bool(v) => Operand::Const(MirConst::Bool(*v)),
            HirLiteral::Char(c) => Operand::Const(MirConst::Char(*c)),
            HirLiteral::String(s) => Operand::Const(MirConst::String(s.clone())),
            HirLiteral::Null => Operand::Const(MirConst::Null),
        }
    }

    fn lower_variable(&self, name: &str, b: &FnBuilder) -> Operand {
        let local = b
            .lookup(name)
            .unwrap_or_else(|| panic!("unbound variable `{}`", name));
        Operand::Copy(local)
    }

    fn lower_binop(
        &mut self,
        l: &TypedExpr,
        op: &BinaryOperator,
        r: &TypedExpr,
        result_ty: &ResolvedType,
        subst: &Subst,
        b: &mut FnBuilder,
    ) -> Operand {
        let lop = self.lower_expr(l, subst, b);
        let rop = self.lower_expr(r, subst, b);
        let mty = self.lower_type(result_ty, subst);
        let tmp = b.new_temp(mty);
        b.push_stmt(Stmt::Assign(tmp, AssignValue::Bin(map_binop(op), lop, rop)));
        Operand::Copy(tmp)
    }

    fn lower_unop(
        &mut self,
        op: &UnaryOperator,
        e: &TypedExpr,
        result_ty: &ResolvedType,
        subst: &Subst,
        b: &mut FnBuilder,
    ) -> Operand {
        let inner = self.lower_expr(e, subst, b);
        let muop = match op {
            UnaryOperator::Neg => UnOp::Neg,
            UnaryOperator::Not => UnOp::Not,
        };
        let mty = self.lower_type(result_ty, subst);
        let tmp = b.new_temp(mty);
        b.push_stmt(Stmt::Assign(tmp, AssignValue::Un(muop, inner)));
        Operand::Copy(tmp)
    }

    fn lower_call(
        &mut self,
        callee: &TypedExpr,
        type_args: &[ResolvedType],
        args: &[TypedExpr],
        result_ty: &ResolvedType,
        subst: &Subst,
        b: &mut FnBuilder,
    ) -> Operand {
        let (fn_id, recv_op, owner_args) = match &callee.kind {
            ExprKind::Variable(name) => {
                let fn_id = self
                    .lookup_free_function(name)
                    .unwrap_or_else(|| panic!("unknown function `{}`", name));
                (fn_id, None, vec![])
            }
            ExprKind::Member(recv, method) => {
                let recv_ty = subst.apply(&recv.ty);
                let recv_op = self.lower_expr(recv, subst, b);
                match recv_ty {
                    ResolvedType::Struct(owner_id, owner_args) => {
                        let fn_id = self.resolve_method(owner_id, &owner_args, method);
                        (fn_id, Some(recv_op), owner_args)
                    }
                    ResolvedType::Interface(_, _) => {
                        unimplemented!("Phase 2: virtual dispatch")
                    }
                    other => panic!("method call on non-struct: {:?}", other),
                }
            }
            other => panic!("unsupported callee shape: {:?}", other),
        };

        let ta_concrete: Vec<ResolvedType> = type_args.iter().map(|t| subst.apply(t)).collect();
        let mut all_args = owner_args;
        all_args.extend(ta_concrete);
        let target = self.mono_fn(fn_id, all_args);

        let mut arg_ops: Vec<Operand> = vec![];
        if let Some(op) = recv_op {
            arg_ops.push(op);
        }
        for a in args {
            arg_ops.push(self.lower_expr(a, subst, b));
        }

        let result_mir = lower_value_type(self, result_ty, subst);
        let tmp = b.new_temp(result_mir);
        b.push_stmt(Stmt::Assign(
            tmp,
            AssignValue::Call(Callee::Static(target), arg_ops),
        ));
        Operand::Copy(tmp)
    }

    fn lower_struct_init(
        &mut self,
        id: TypeId,
        type_args: &[ResolvedType],
        fields: &[(String, TypedExpr)],
        subst: &Subst,
        b: &mut FnBuilder,
    ) -> Operand {
        let concrete: Vec<ResolvedType> = type_args.iter().map(|a| subst.apply(a)).collect();
        let mir_struct = self.get_or_create_struct(id, concrete);

        let decl_order: Vec<String> = match &self.mir.types[&mir_struct] {
            MirTypeDef::Struct { fields, .. } => fields.iter().map(|f| f.name.clone()).collect(),
            _ => unreachable!(),
        };

        // Lower in source order so side effects fire as written, then reorder.
        let mut by_name: HashMap<String, Operand> = HashMap::new();
        for (name, e) in fields {
            let op = self.lower_expr(e, subst, b);
            by_name.insert(name.clone(), op);
        }
        let ordered: Vec<Operand> = decl_order
            .iter()
            .map(|n| {
                by_name
                    .remove(n)
                    .unwrap_or_else(|| panic!("missing field `{}` in init", n))
            })
            .collect();

        let mty = MirType::ManagedRef(mir_struct);
        let tmp = b.new_temp(mty);
        b.push_stmt(Stmt::Assign(
            tmp,
            AssignValue::AllocStruct(mir_struct, ordered),
        ));
        Operand::Copy(tmp)
    }

    fn lower_member(
        &mut self,
        recv: &TypedExpr,
        field: &str,
        result_ty: &ResolvedType,
        subst: &Subst,
        b: &mut FnBuilder,
    ) -> Operand {
        let recv_op = self.lower_expr(recv, subst, b);
        let recv_ty = subst.apply(&recv.ty);
        let mir_struct = match recv_ty {
            ResolvedType::Struct(id, args) => self.get_or_create_struct(id, args),
            other => panic!("member access on non-struct: {:?}", other),
        };
        let idx = match &self.mir.types[&mir_struct] {
            MirTypeDef::Struct { fields, .. } => fields
                .iter()
                .position(|f| f.name == field)
                .unwrap_or_else(|| panic!("no field `{}`", field)) as u32,
            _ => unreachable!(),
        };
        let result_mir = self.lower_type(result_ty, subst);
        let tmp = b.new_temp(result_mir);
        b.push_stmt(Stmt::Assign(tmp, AssignValue::Field(recv_op, idx)));
        Operand::Copy(tmp)
    }

    fn lower_as(
        &mut self,
        e: &TypedExpr,
        target: &ResolvedType,
        _result_ty: &ResolvedType,
        subst: &Subst,
        b: &mut FnBuilder,
    ) -> Operand {
        let src_ty = subst.apply(&e.ty);
        let target_ty = subst.apply(target);

        match (&src_ty, &target_ty) {
            (ResolvedType::Primitive(_), ResolvedType::Primitive(_)) => {
                let op = self.lower_expr(e, subst, b);
                let mty = self.lower_type(&target_ty, subst);
                let tmp = b.new_temp(mty.clone());
                b.push_stmt(Stmt::Assign(tmp, AssignValue::Cast(op, mty)));
                Operand::Copy(tmp)
            }
            _ => unimplemented!("Phase 2: as on {:?} -> {:?}", src_ty, target_ty),
        }
    }

    fn lower_if(
        &mut self,
        cond: &TypedExpr,
        then_blk: &HirBlock,
        else_blk: Option<&HirBlock>,
        result_ty: &ResolvedType,
        subst: &Subst,
        b: &mut FnBuilder,
    ) -> Operand {
        let cond_op = self.lower_expr(cond, subst, b);

        let then_bb = b.new_block();
        let else_bb = b.new_block();
        let join_bb = b.new_block();

        let result_mir = lower_value_type(self, result_ty, subst);
        let result = b.new_temp(result_mir);

        b.terminate(Terminator::CondBr(cond_op, then_bb, else_bb));

        b.switch_to(then_bb);
        let t_val = self.lower_block(then_blk, subst, b);
        // After lowering the arm, current_block may have been split by nested
        // control flow. Terminate only the still-open tail.
        if matches!(
            b.blocks[&b.current_block].terminator,
            Terminator::Unreachable
        ) {
            if let Some(op) = t_val {
                b.push_stmt(Stmt::Assign(result, AssignValue::Use(op)));
            }
            b.terminate(Terminator::Goto(join_bb));
        }

        b.switch_to(else_bb);
        if let Some(eb) = else_blk {
            let e_val = self.lower_block(eb, subst, b);
            if matches!(
                b.blocks[&b.current_block].terminator,
                Terminator::Unreachable
            ) {
                if let Some(op) = e_val {
                    b.push_stmt(Stmt::Assign(result, AssignValue::Use(op)));
                }
                b.terminate(Terminator::Goto(join_bb));
            }
        } else {
            b.terminate(Terminator::Goto(join_bb));
        }

        b.switch_to(join_bb);
        Operand::Copy(result)
    }

    fn lower_block_expr(
        &mut self,
        block: &HirBlock,
        _result_ty: &ResolvedType,
        subst: &Subst,
        b: &mut FnBuilder,
    ) -> Operand {
        match self.lower_block(block, subst, b) {
            Some(op) => op,
            None => Operand::Const(MirConst::Null),
        }
    }

    fn lower_list_lit(
        &mut self,
        items: &[TypedExpr],
        result_ty: &ResolvedType,
        subst: &Subst,
        b: &mut FnBuilder,
    ) -> Operand {
        let elem_ty = match subst.apply(result_ty) {
            ResolvedType::Struct(_, args) if args.len() == 1 => args[0].clone(),
            other => panic!("list literal with non-List<T> type: {:?}", other),
        };
        let elem_mir = self.lower_concrete_type(&elem_ty);

        let ops: Vec<Operand> = items.iter().map(|e| self.lower_expr(e, subst, b)).collect();

        let result_mir = self.lower_type(result_ty, subst);
        let tmp = b.new_temp(result_mir);
        b.push_stmt(Stmt::Assign(tmp, AssignValue::AllocList(elem_mir, ops)));
        Operand::Copy(tmp)
    }

    fn lower_map_lit(
        &mut self,
        pairs: &[(TypedExpr, TypedExpr)],
        result_ty: &ResolvedType,
        subst: &Subst,
        b: &mut FnBuilder,
    ) -> Operand {
        let (k_ty, v_ty) = match subst.apply(result_ty) {
            ResolvedType::Struct(_, args) if args.len() == 2 => (args[0].clone(), args[1].clone()),
            other => panic!("map literal with non-Map<K,V> type: {:?}", other),
        };
        let k_mir = self.lower_concrete_type(&k_ty);
        let v_mir = self.lower_concrete_type(&v_ty);

        let ops: Vec<(Operand, Operand)> = pairs
            .iter()
            .map(|(k, v)| (self.lower_expr(k, subst, b), self.lower_expr(v, subst, b)))
            .collect();

        let result_mir = self.lower_type(result_ty, subst);
        let tmp = b.new_temp(result_mir);
        b.push_stmt(Stmt::Assign(tmp, AssignValue::AllocMap(k_mir, v_mir, ops)));
        Operand::Copy(tmp)
    }

    pub fn lookup_free_function(&self, name: &str) -> Option<FnId> {
        for (id, f) in &self.hir.functions {
            if f.owner.is_none() && f.name == name {
                return Some(*id);
            }
        }
        None
    }

    /// Phase 1 method resolution: inherent methods, plus universal-extend
    /// entries whose target args are all type-params. Specialised extends
    /// (`extend Foo<i32>`) require unification — deferred to Phase 2.
    pub fn resolve_method(
        &self,
        owner: TypeId,
        _owner_args: &[ResolvedType],
        method_name: &str,
    ) -> FnId {
        let s = &self.hir.structs[&owner];
        for fn_id in &s.methods {
            if self.hir.functions[fn_id].name == method_name {
                return *fn_id;
            }
        }
        for (target_args, fn_id) in &s.specialised_methods {
            if self.hir.functions[fn_id].name != method_name {
                continue;
            }
            let all_params = target_args
                .iter()
                .all(|a| matches!(a, ResolvedType::TypeParam(_)));
            if all_params {
                return *fn_id;
            }
        }
        panic!("no method `{}` on `{}`", method_name, s.name);
    }
}

fn prim_of(ty: &ResolvedType) -> PrimitiveType {
    match ty {
        ResolvedType::Primitive(p) => p.clone(),
        _ => panic!("non-primitive type on a numeric literal"),
    }
}

fn map_binop(op: &BinaryOperator) -> BinOp {
    use BinaryOperator::*;
    match op {
        Add => BinOp::Add,
        Sub => BinOp::Sub,
        Mul => BinOp::Mul,
        Div => BinOp::Div,
        Mod => BinOp::Mod,
        And => BinOp::And,
        Or => BinOp::Or,
        Eq => BinOp::Eq,
        Neq => BinOp::Neq,
        Lt => BinOp::Lt,
        Le => BinOp::Le,
        Gt => BinOp::Gt,
        Ge => BinOp::Ge,
    }
}

// `Null` is how the validator types void returns. The MIR has no Unit, so
// at value-producing positions (call results, if-result temps) we fall back
// to a Bool placeholder until a real Unit type lands.
fn lower_value_type(lower: &mut Lower, ty: &ResolvedType, subst: &Subst) -> MirType {
    let ty = subst.apply(ty);
    if matches!(ty, ResolvedType::Null) {
        return MirType::Primitive(PrimitiveType::Bool);
    }
    lower.lower_concrete_type(&ty)
}
