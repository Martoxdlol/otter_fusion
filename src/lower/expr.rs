use crate::{
    hir::{
        BinaryOperator, ExprKind, FnId, HirBlock, HirLiteral, PrimitiveType, ResolvedType, TypeId,
        TypeParamId, TypedExpr, UnaryOperator,
    },
    lower::{Lower, builder::FnBuilder, subst::Subst},
    mir::{
        AssignValue, BinOp, Callee, MirConst, MirType, MirTypeDef, MirTypeId, Operand, Stmt,
        Terminator, TrapReason, UnOp,
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
            ExprKind::Is(e, target) => self.lower_is(e, target, subst, b),
            ExprKind::If(c, t, e) => self.lower_if(c, t, e.as_deref(), &expr.ty, subst, b),
            ExprKind::Block(blk) => self.lower_block_expr(blk, &expr.ty, subst, b),
            ExprKind::LiteralList(items) => self.lower_list_lit(items, &expr.ty, subst, b),
            ExprKind::LiteralMap(pairs) => self.lower_map_lit(pairs, &expr.ty, subst, b),
            // The validator currently replaces lambda bodies with a null
            // literal of `Function` type, so a real FunctionLiteral never
            // reaches lowering. If that changes, this is where closure
            // env synthesis would go.
            ExprKind::FunctionLiteral(..) => {
                panic!("FunctionLiteral in HIR — validator should have erased the body")
            }
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
        let from_l = subst.apply(&l.ty);
        let from_r = subst.apply(&r.ty);

        // For comparisons the result is `bool`; pick the wider operand
        // type as the common operand type. For arithmetic / logical ops
        // the result type *is* the operand type.
        let result_concrete = subst.apply(result_ty);
        let common_ty: ResolvedType = match op {
            BinaryOperator::Eq
            | BinaryOperator::Neq
            | BinaryOperator::Lt
            | BinaryOperator::Le
            | BinaryOperator::Gt
            | BinaryOperator::Ge => {
                // Promote both to the wider primitive (favour left when equal).
                if let (
                    ResolvedType::Primitive(pl),
                    ResolvedType::Primitive(pr),
                ) = (&from_l, &from_r)
                {
                    pick_wider_primitive(pl, pr)
                } else {
                    from_l.clone()
                }
            }
            _ => result_concrete.clone(),
        };

        let lop = self.coerce_to(lop, &from_l, &common_ty, b);
        let rop = self.coerce_to(rop, &from_r, &common_ty, b);
        let mty = self.lower_concrete_type(&result_concrete);
        b.emit(AssignValue::Bin(map_binop(op), lop, rop), mty)
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
        b.emit(AssignValue::Un(muop, inner), mty)
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
        let result_mir = self.lower_type(result_ty, subst);

        // Virtual interface dispatch.
        if let ExprKind::Member(recv, method) = &callee.kind {
            let recv_ty = subst.apply(&recv.ty);
            if let ResolvedType::Interface(iface_id, iface_args) = recv_ty.clone() {
                let recv_op = self.lower_expr(recv, subst, b);
                let iface_concrete: Vec<ResolvedType> =
                    iface_args.iter().map(|a| subst.apply(a)).collect();
                let iface_mir =
                    self.get_or_create_interface(iface_id, iface_concrete);
                let (decl_iface, slot) = self
                    .find_method_slot(iface_mir, method)
                    .unwrap_or_else(|| panic!("no method `{}` on interface", method));

                // Lower args directly (no coercion against a static param
                // list — codegen reads the signature from the vtable slot).
                let mut arg_ops: Vec<Operand> = vec![recv_op.clone()];
                for a in args {
                    arg_ops.push(self.lower_expr(a, subst, b));
                }
                return b.emit(
                    AssignValue::Call(Callee::Virtual(recv_op, decl_iface, slot), arg_ops),
                    result_mir,
                );
            }
        }

        // Indirect call: callee value is of function type.
        let callee_ty = subst.apply(&callee.ty);
        let static_callee_var = match &callee.kind {
            ExprKind::Variable(name) => self.lookup_free_function(name),
            _ => None,
        };
        let is_member_struct = matches!(
            &callee.kind,
            ExprKind::Member(recv, _) if matches!(subst.apply(&recv.ty), ResolvedType::Struct(_, _))
        );
        let is_static = static_callee_var.is_some() || is_member_struct;
        if !is_static && matches!(callee_ty, ResolvedType::Function(_, _)) {
            let callee_op = self.lower_expr(callee, subst, b);
            let arg_ops: Vec<Operand> = args
                .iter()
                .map(|a| self.lower_expr(a, subst, b))
                .collect();
            return b.emit(
                AssignValue::Call(Callee::Indirect(callee_op), arg_ops),
                result_mir,
            );
        }

        let (fn_id, recv_op, owner_args, method_resolved_args) = match &callee.kind {
            ExprKind::Variable(name) => {
                let fn_id = self
                    .lookup_free_function(name)
                    .unwrap_or_else(|| panic!("unknown function `{}`", name));
                (fn_id, None, vec![], vec![])
            }
            ExprKind::Member(recv, method) => {
                let recv_ty = subst.apply(&recv.ty);
                let recv_op = self.lower_expr(recv, subst, b);
                match recv_ty {
                    ResolvedType::Struct(owner_id, owner_args) => {
                        let (fn_id, method_args) =
                            self.resolve_method_with_args(owner_id, &owner_args, method);
                        (fn_id, Some(recv_op), owner_args, method_args)
                    }
                    other => panic!("method call on non-struct: {:?}", other),
                }
            }
            other => panic!("unsupported callee shape: {:?}", other),
        };

        // type_args = [owner_args..., method_resolved_args..., explicit_call_args...]
        // The first two segments bind the owner's params + the method's own
        // (resolved from a universal extend, if any). The last segment is
        // what the user wrote at the call site.
        let ta_concrete: Vec<ResolvedType> = type_args.iter().map(|t| subst.apply(t)).collect();
        let mut all_args = owner_args;
        all_args.extend(method_resolved_args);
        all_args.extend(ta_concrete);
        let target = self.mono_fn(fn_id, all_args.clone());

        let callee = self.hir.functions[&fn_id].clone();
        let callee_subst = Subst::new(callee.type_params.clone(), all_args);

        let mut arg_ops: Vec<Operand> = vec![];
        if let Some(op) = recv_op {
            arg_ops.push(op);
        }
        for (a, p) in args.iter().zip(callee.params.iter()) {
            let raw = self.lower_expr(a, subst, b);
            let from = subst.apply(&a.ty);
            let to = callee_subst.apply(&p.ty);
            arg_ops.push(self.coerce_to(raw, &from, &to, b));
        }

        b.emit(AssignValue::Call(Callee::Static(target), arg_ops), result_mir)
    }

    fn find_method_slot(&self, iface_mir: MirTypeId, name: &str) -> Option<(MirTypeId, u32)> {
        if let MirTypeDef::Interface {
            method_slots,
            extends,
            ..
        } = &self.mir.types[&iface_mir]
        {
            if let Some(i) = method_slots.iter().position(|s| s.name == name) {
                return Some((iface_mir, i as u32));
            }
            let parents = extends.clone();
            for p in parents {
                if let Some(found) = self.find_method_slot(p, name) {
                    return Some(found);
                }
            }
        }
        None
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
        let mir_struct = self.get_or_create_struct(id, concrete.clone());

        let decl_order: Vec<String> = match &self.mir.types[&mir_struct] {
            MirTypeDef::Struct { fields, .. } => fields.iter().map(|f| f.name.clone()).collect(),
            _ => unreachable!(),
        };

        // Map field name → declared (post-subst) ResolvedType, for widening.
        let hir_struct = self.hir.structs[&id].clone();
        let field_subst = Subst::new(hir_struct.type_params.clone(), concrete);
        let field_decl_ty: HashMap<String, ResolvedType> = hir_struct
            .fields
            .iter()
            .map(|f| (f.name.clone(), field_subst.apply(&f.ty)))
            .collect();

        // Lower in source order so side effects fire as written, then reorder.
        let mut by_name: HashMap<String, Operand> = HashMap::new();
        for (name, e) in fields {
            let raw = self.lower_expr(e, subst, b);
            let from = subst.apply(&e.ty);
            let to = field_decl_ty
                .get(name)
                .unwrap_or_else(|| panic!("unknown field `{}` in init", name));
            let op = self.coerce_to(raw, &from, to, b);
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
        b.emit(AssignValue::AllocStruct(mir_struct, ordered), mty)
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
        b.emit(AssignValue::Field(recv_op, idx), result_mir)
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

        if src_ty == target_ty {
            return self.lower_expr(e, subst, b);
        }

        match (&src_ty, &target_ty) {
            (ResolvedType::Primitive(_), ResolvedType::Primitive(_)) => {
                let op = self.lower_expr(e, subst, b);
                let mty = self.lower_type(&target_ty, subst);
                b.emit(AssignValue::Cast(op, mty.clone()), mty)
            }
            // Variant -> union (widen). Reuse the existing widen helper.
            (_, ResolvedType::Union(_)) => {
                let op = self.lower_expr(e, subst, b);
                self.coerce_to(op, &src_ty, &target_ty, b)
            }
            // Union -> variant (narrow).
            (ResolvedType::Union(variants), narrowed) => {
                let scrutinee = self.lower_expr(e, subst, b);
                self.lower_union_narrow(scrutinee, variants, narrowed, subst, b)
            }
            // Struct -> interface (upcast). No-op rebind; same pointer shape.
            (ResolvedType::Struct(_, _), ResolvedType::Interface(_, _)) => {
                let op = self.lower_expr(e, subst, b);
                let mty = self.lower_type(&target_ty, subst);
                b.emit(AssignValue::Use(op), mty)
            }
            // Interface -> interface (parent / sibling) — also no-op.
            (ResolvedType::Interface(_, _), ResolvedType::Interface(_, _)) => {
                let op = self.lower_expr(e, subst, b);
                let mty = self.lower_type(&target_ty, subst);
                b.emit(AssignValue::Use(op), mty)
            }
            // Null literal -> nullable/union (the coerce_to helper handles
            // both NullableRef and tagged-union construction).
            (ResolvedType::Null, _) => {
                let op = self.lower_expr(e, subst, b);
                self.coerce_to(op, &src_ty, &target_ty, b)
            }
            _ => panic!("unsupported `as` {:?} -> {:?}", src_ty, target_ty),
        }
    }

    fn lower_is(
        &mut self,
        e: &TypedExpr,
        target: &ResolvedType,
        subst: &Subst,
        b: &mut FnBuilder,
    ) -> Operand {
        let src_ty = subst.apply(&e.ty);
        let target_ty = subst.apply(target);
        let scrutinee = self.lower_expr(e, subst, b);

        // NullableRef path.
        if let ResolvedType::Union(variants) = &src_ty
            && self.try_nullable_ref(variants).is_some() {
                let op = if matches!(target_ty, ResolvedType::Null) {
                    BinOp::Eq
                } else {
                    BinOp::Neq
                };
                return b.emit(
                    AssignValue::Bin(op, scrutinee, Operand::Const(MirConst::Null)),
                    MirType::Primitive(PrimitiveType::Bool),
                );
            }

        // Tagged union path.
        let mid = match &src_ty {
            ResolvedType::Union(vs) => self.get_or_create_union(vs.clone()),
            other => panic!("`is` scrutinee must be a union: got {:?}", other),
        };
        let target_mir = self.lower_type(&target_ty, subst);
        let target_tag = self.tag_of_variant_mir(mid, &target_mir);

        let tag_tmp = b.emit(
            AssignValue::UnionTag(scrutinee),
            MirType::Primitive(PrimitiveType::Uint16),
        );
        b.emit(
            AssignValue::Bin(
                BinOp::Eq,
                tag_tmp,
                Operand::Const(MirConst::Int(target_tag as i64, PrimitiveType::Uint16)),
            ),
            MirType::Primitive(PrimitiveType::Bool),
        )
    }

    pub fn lower_union_narrow(
        &mut self,
        scrutinee: Operand,
        variants: &[ResolvedType],
        narrowed_ty: &ResolvedType,
        subst: &Subst,
        b: &mut FnBuilder,
    ) -> Operand {
        // NullableRef path: T | null narrowed to T or null.
        if self.try_nullable_ref(variants).is_some() {
            return self.emit_nullable_narrow(scrutinee, narrowed_ty, subst, b);
        }
        // Tagged path.
        let mid = self.get_or_create_union(variants.to_vec());
        let target_mir = self.lower_type(narrowed_ty, subst);
        let target_tag = self.tag_of_variant_mir(mid, &target_mir);

        let tag_tmp = b.emit(
            AssignValue::UnionTag(scrutinee.clone()),
            MirType::Primitive(PrimitiveType::Uint16),
        );
        let ok = b.emit(
            AssignValue::Bin(
                BinOp::Eq,
                tag_tmp,
                Operand::Const(MirConst::Int(target_tag as i64, PrimitiveType::Uint16)),
            ),
            MirType::Primitive(PrimitiveType::Bool),
        );

        let ok_bb = b.new_block();
        let trap_bb = b.new_block();
        let join_bb = b.new_block();
        b.terminate(Terminator::CondBr(ok, ok_bb, trap_bb));

        let result = b.new_temp(target_mir.clone());

        b.switch_to(ok_bb);
        b.push_stmt(Stmt::Assign(
            result,
            AssignValue::UnionPayload(scrutinee, mid),
        ));
        b.terminate(Terminator::Goto(join_bb));

        b.switch_to(trap_bb);
        b.terminate(Terminator::Trap(TrapReason::AsMismatch));

        b.switch_to(join_bb);
        Operand::Copy(result)
    }

    fn emit_nullable_narrow(
        &mut self,
        scrutinee: Operand,
        narrowed_ty: &ResolvedType,
        subst: &Subst,
        b: &mut FnBuilder,
    ) -> Operand {
        let expect_null = matches!(narrowed_ty, ResolvedType::Null);
        let cmp_op = if expect_null { BinOp::Eq } else { BinOp::Neq };
        let cmp = b.emit(
            AssignValue::Bin(cmp_op, scrutinee.clone(), Operand::Const(MirConst::Null)),
            MirType::Primitive(PrimitiveType::Bool),
        );

        let ok_bb = b.new_block();
        let trap_bb = b.new_block();
        let join_bb = b.new_block();
        b.terminate(Terminator::CondBr(cmp, ok_bb, trap_bb));

        let target_mir = self.lower_type(narrowed_ty, subst);
        let result = b.new_temp(target_mir);

        b.switch_to(trap_bb);
        b.terminate(Terminator::Trap(TrapReason::AsMismatch));

        b.switch_to(ok_bb);
        if expect_null {
            b.push_stmt(Stmt::Assign(
                result,
                AssignValue::Use(Operand::Const(MirConst::Null)),
            ));
        } else {
            b.push_stmt(Stmt::Assign(result, AssignValue::Use(scrutinee)));
        }
        b.terminate(Terminator::Goto(join_bb));

        b.switch_to(join_bb);
        Operand::Copy(result)
    }

    fn tag_of_variant_mir(&self, union_mid: MirTypeId, variant: &MirType) -> u16 {
        match &self.mir.types[&union_mid] {
            MirTypeDef::Union { variants, .. } => variants
                .iter()
                .find(|v| &v.ty == variant)
                .map(|v| v.tag)
                .unwrap_or_else(|| panic!("variant not in union: {:?}", variant)),
            _ => unreachable!(),
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

        let result_mir = self.lower_type(result_ty, subst);
        let result = b.new_temp(result_mir);

        b.terminate(Terminator::CondBr(cond_op, then_bb, else_bb));

        let target_ty = subst.apply(result_ty);

        b.switch_to(then_bb);
        let t_src_ty = then_blk.returns.as_ref().map(|e| subst.apply(&e.ty));
        let t_val = self.lower_block(then_blk, subst, b);
        // After lowering the arm, current_block may have been split by nested
        // control flow. Touch only the still-open tail.
        if b.is_open() {
            if let (Some(op), Some(src)) = (t_val, t_src_ty) {
                let coerced = self.coerce_to(op, &src, &target_ty, b);
                b.push_stmt(Stmt::Assign(result, AssignValue::Use(coerced)));
            }
            b.terminate(Terminator::Goto(join_bb));
        }

        b.switch_to(else_bb);
        if let Some(eb) = else_blk {
            let e_src_ty = eb.returns.as_ref().map(|e| subst.apply(&e.ty));
            let e_val = self.lower_block(eb, subst, b);
            if b.is_open() {
                if let (Some(op), Some(src)) = (e_val, e_src_ty) {
                    let coerced = self.coerce_to(op, &src, &target_ty, b);
                    b.push_stmt(Stmt::Assign(result, AssignValue::Use(coerced)));
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

        let ops: Vec<Operand> = items
            .iter()
            .map(|e| {
                let raw = self.lower_expr(e, subst, b);
                let from = subst.apply(&e.ty);
                self.coerce_to(raw, &from, &elem_ty, b)
            })
            .collect();

        let result_mir = self.lower_type(result_ty, subst);
        b.emit(AssignValue::AllocList(elem_mir, ops), result_mir)
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
            .map(|(k, v)| {
                let k_raw = self.lower_expr(k, subst, b);
                let k_from = subst.apply(&k.ty);
                let k_op = self.coerce_to(k_raw, &k_from, &k_ty, b);
                let v_raw = self.lower_expr(v, subst, b);
                let v_from = subst.apply(&v.ty);
                let v_op = self.coerce_to(v_raw, &v_from, &v_ty, b);
                (k_op, v_op)
            })
            .collect();

        let result_mir = self.lower_type(result_ty, subst);
        b.emit(AssignValue::AllocMap(k_mir, v_mir, ops), result_mir)
    }

    pub fn lookup_free_function(&self, name: &str) -> Option<FnId> {
        for (id, f) in &self.hir.functions {
            if f.owner.is_none() && f.name == name {
                return Some(*id);
            }
        }
        None
    }

    /// Resolve `OwnerStruct<owner_args>::method_name` to a concrete
    /// `(FnId, type_args)` pair, where `type_args` is the binding for the
    /// method's own `type_params` (caller still appends explicit method
    /// type-arguments).
    ///
    /// Lookup order:
    ///   1. Inherent methods (declared on the struct itself).
    ///   2. Specialised extends with target_args that match `owner_args`
    ///      exactly.
    ///   3. Universal extends (`extend<T> Owner<T>`) — unify target_args
    ///      against `owner_args` to bind the extend's type params.
    pub fn resolve_method(
        &self,
        owner: TypeId,
        owner_args: &[ResolvedType],
        method_name: &str,
    ) -> FnId {
        self.resolve_method_with_args(owner, owner_args, method_name).0
    }

    pub fn resolve_method_with_args(
        &self,
        owner: TypeId,
        owner_args: &[ResolvedType],
        method_name: &str,
    ) -> (FnId, Vec<ResolvedType>) {
        let s = &self.hir.structs[&owner];

        // 1. Inherent methods. Their type_params are method-level only.
        for fn_id in &s.methods {
            if self.hir.functions[fn_id].name == method_name {
                return (*fn_id, vec![]);
            }
        }

        let specialised: Vec<(Vec<ResolvedType>, FnId)> = s.specialised_methods.clone();
        // 2. Specialised extend matching owner_args literally.
        for (target_args, fn_id) in &specialised {
            if self.hir.functions[fn_id].name != method_name {
                continue;
            }
            if target_args == owner_args {
                return (*fn_id, vec![]);
            }
        }
        // 3. Universal extend (each target_arg is a TypeParam).
        for (target_args, fn_id) in &specialised {
            if self.hir.functions[fn_id].name != method_name {
                continue;
            }
            let all_params = target_args
                .iter()
                .all(|a: &ResolvedType| matches!(a, ResolvedType::TypeParam(_)));
            if !all_params || target_args.len() != owner_args.len() {
                continue;
            }
            // Bind each extend type param to the concrete owner arg in
            // the same position. Method's own type_params are exactly
            // these extend params (in declaration order), so we can
            // return owner_args directly.
            let f = &self.hir.functions[fn_id];
            let extend_param_to_arg: std::collections::HashMap<TypeParamId, ResolvedType> = target_args
                .iter()
                .zip(owner_args.iter())
                .map(|(tp, a)| match tp {
                    ResolvedType::TypeParam(id) => (*id, a.clone()),
                    _ => unreachable!(),
                })
                .collect();
            let method_args: Vec<ResolvedType> = f
                .type_params
                .iter()
                .map(|p| {
                    extend_param_to_arg
                        .get(p)
                        .cloned()
                        .unwrap_or(ResolvedType::TypeParam(*p))
                })
                .collect();
            return (*fn_id, method_args);
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

fn pick_wider_primitive(a: &PrimitiveType, b: &PrimitiveType) -> ResolvedType {
    use PrimitiveType::*;
    fn rank(p: &PrimitiveType) -> u32 {
        match p {
            Bool => 1,
            Int8 | Uint8 => 8,
            Int16 | Uint16 => 16,
            Int32 | Uint32 | Char => 32,
            Float32 => 33,
            Int64 | Uint64 | String => 64,
            Float64 => 65,
        }
    }
    if rank(a) >= rank(b) {
        ResolvedType::Primitive(a.clone())
    } else {
        ResolvedType::Primitive(b.clone())
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

