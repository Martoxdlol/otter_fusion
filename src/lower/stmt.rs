#![allow(clippy::too_many_arguments)]

use crate::{
    hir::{HirBlock, HirStatement, PrimitiveType, ResolvedType, TypedExpr},
    lower::{Lower, builder::FnBuilder, subst::Subst},
    mir::{
        AssignValue, BinOp, Callee, MirConst, MirType, MirTypeDef, NULL_TAG, Operand, Stmt,
        Terminator,
    },
};

impl Lower {
    pub fn lower_stmt(&mut self, s: &HirStatement, subst: &Subst, b: &mut FnBuilder) {
        match s {
            HirStatement::VarDecl(name, ty, init) => {
                let mty = self.lower_type(ty, subst);
                let local = b.new_local(Some(name.clone()), mty);
                if let Some(e) = init {
                    let raw = self.lower_expr(e, subst, b);
                    let op = self.coerce_to_subst(raw, &e.ty, ty, subst, b);
                    b.push_stmt(Stmt::Assign(local, AssignValue::Use(op)));
                }
                b.bind(name.clone(), local);
            }

            HirStatement::Return(opt_e) => {
                let op = if let Some(e) = opt_e {
                    let raw = self.lower_expr(e, subst, b);
                    let target = self
                        .current_return_type
                        .clone()
                        .expect("return outside lower_function");
                    let from = subst.apply(&e.ty);
                    Some(self.coerce_to(raw, &from, &target, b))
                } else {
                    None
                };
                b.terminate(Terminator::Return(op));
                // Subsequent stmts are unreachable; start a new block so
                // they have somewhere to go.
                let dead = b.new_block();
                b.switch_to(dead);
            }

            HirStatement::Expr(e) => {
                // Lower the expression, discard the result.
                let _ = self.lower_expr(e, subst, b);
            }

            HirStatement::While(cond, body) => {
                self.lower_while(cond, body, subst, b);
            }

            HirStatement::For(name, iter_e, body) => {
                self.lower_for(name, iter_e, body, subst, b);
            }

            HirStatement::Break => {
                let (_cont, brk) = *b.loop_stack.last().expect("validator missed this");
                b.terminate(Terminator::Goto(brk));
                let dead = b.new_block();
                b.switch_to(dead);
            }
            HirStatement::Continue => {
                let (cont, _brk) = *b.loop_stack.last().expect("validator missed this");
                b.terminate(Terminator::Goto(cont));
                let dead = b.new_block();
                b.switch_to(dead);
            }
        }
    }

    fn lower_for(
        &mut self,
        var_name: &str,
        iter_e: &TypedExpr,
        body: &HirBlock,
        subst: &Subst,
        b: &mut FnBuilder,
    ) {
        let iter_ty = subst.apply(&iter_e.ty);
        // List/Map fast path.
        if let ResolvedType::Struct(id, args) = &iter_ty {
            let is_core = self
                .hir
                .modules
                .get(&self.hir.structs[id].module)
                .map(|m| m.name == "of:core")
                .unwrap_or(false);
            let name = self.hir.structs[id].name.clone();
            if is_core && name == "List" && args.len() == 1 {
                self.lower_for_list(var_name, iter_e, &iter_ty, args[0].clone(), body, subst, b);
                return;
            }
            if is_core && name == "Map" && args.len() == 2 {
                self.lower_for_map(
                    var_name,
                    iter_e,
                    &iter_ty,
                    args[0].clone(),
                    args[1].clone(),
                    body,
                    subst,
                    b,
                );
                return;
            }
        }
        self.lower_for_iterator(var_name, iter_e, &iter_ty, body, subst, b);
    }

    fn lower_for_list(
        &mut self,
        var_name: &str,
        iter_e: &TypedExpr,
        list_ty: &ResolvedType,
        elem_ty: ResolvedType,
        body: &HirBlock,
        subst: &Subst,
        b: &mut FnBuilder,
    ) {
        let (list_id, list_args) = match list_ty {
            ResolvedType::Struct(id, a) => (*id, a.clone()),
            _ => unreachable!(),
        };
        let list_op = self.lower_expr(iter_e, subst, b);
        let list_mir = self.lower_concrete_type(list_ty);
        let it_local = b.new_local(Some("__it".to_string()), list_mir);
        b.push_stmt(Stmt::Assign(it_local, AssignValue::Use(list_op)));

        let (size_fn, size_extra) = self.resolve_method_with_args(list_id, &list_args, "size");
        let mut size_args = list_args.clone();
        size_args.extend(size_extra);
        let size_mir_fn = self.mono_fn(size_fn, size_args);
        let n_local = b.new_temp(MirType::Primitive(PrimitiveType::Int64));
        b.push_stmt(Stmt::Assign(
            n_local,
            AssignValue::Call(Callee::Static(size_mir_fn), vec![Operand::Copy(it_local)]),
        ));

        let i_local = b.new_local(Some("__i".to_string()), MirType::Primitive(PrimitiveType::Int64));
        b.push_stmt(Stmt::Assign(
            i_local,
            AssignValue::Use(Operand::Const(MirConst::Int(0, PrimitiveType::Int64))),
        ));

        let head_bb = b.new_block();
        let body_bb = b.new_block();
        let cont_bb = b.new_block();
        let exit_bb = b.new_block();
        b.terminate(Terminator::Goto(head_bb));

        b.switch_to(head_bb);
        let done = b.emit(
            AssignValue::Bin(BinOp::Ge, Operand::Copy(i_local), Operand::Copy(n_local)),
            MirType::Primitive(PrimitiveType::Bool),
        );
        b.terminate(Terminator::CondBr(done, exit_bb, body_bb));

        b.switch_to(body_bb);
        let (get_fn, get_extra) = self.resolve_method_with_args(list_id, &list_args, "get");
        let mut get_args = list_args.clone();
        get_args.extend(get_extra);
        let get_mir_fn = self.mono_fn(get_fn, get_args);
        let nullable_mir = self.maybe_nullable_for(&elem_ty);
        let opt_local = b.new_temp(nullable_mir.clone());
        b.push_stmt(Stmt::Assign(
            opt_local,
            AssignValue::Call(
                Callee::Static(get_mir_fn),
                vec![Operand::Copy(it_local), Operand::Copy(i_local)],
            ),
        ));

        let elem_mir = self.lower_concrete_type(&elem_ty);
        let x_local = b.new_local(Some(var_name.to_string()), elem_mir);
        match &nullable_mir {
            MirType::NullableRef(_) => {
                b.push_stmt(Stmt::Assign(
                    x_local,
                    AssignValue::Use(Operand::Copy(opt_local)),
                ));
            }
            MirType::Union(uid) => {
                b.push_stmt(Stmt::Assign(
                    x_local,
                    AssignValue::UnionPayload(Operand::Copy(opt_local), *uid),
                ));
            }
            _ => unreachable!(),
        }

        b.bind(var_name.to_string(), x_local);
        b.loop_stack.push((cont_bb, exit_bb));
        let _ = self.lower_block(body, subst, b);
        b.loop_stack.pop();
        b.terminate_if_open(Terminator::Goto(cont_bb));

        b.switch_to(cont_bb);
        let next_i = b.emit(
            AssignValue::Bin(
                BinOp::Add,
                Operand::Copy(i_local),
                Operand::Const(MirConst::Int(1, PrimitiveType::Int64)),
            ),
            MirType::Primitive(PrimitiveType::Int64),
        );
        b.push_stmt(Stmt::Assign(i_local, AssignValue::Use(next_i)));
        b.terminate(Terminator::Goto(head_bb));

        b.switch_to(exit_bb);
    }

    fn lower_for_map(
        &mut self,
        var_name: &str,
        iter_e: &TypedExpr,
        map_ty: &ResolvedType,
        k_ty: ResolvedType,
        v_ty: ResolvedType,
        body: &HirBlock,
        subst: &Subst,
        b: &mut FnBuilder,
    ) {
        let (map_id, map_args) = match map_ty {
            ResolvedType::Struct(id, a) => (*id, a.clone()),
            _ => unreachable!(),
        };
        let map_op = self.lower_expr(iter_e, subst, b);
        let map_mir = self.lower_concrete_type(map_ty);
        let it_local = b.new_local(Some("__map".to_string()), map_mir);
        b.push_stmt(Stmt::Assign(it_local, AssignValue::Use(map_op)));

        // keys = map.keys()
        let (keys_fn, keys_extra) = self.resolve_method_with_args(map_id, &map_args, "keys");
        let mut keys_call_args = map_args.clone();
        keys_call_args.extend(keys_extra);
        let keys_mir_fn = self.mono_fn(keys_fn, keys_call_args);
        let keys_ty = ResolvedType::Struct(
            self.hir
                .structs
                .iter()
                .find(|(_, s)| {
                    s.name == "List"
                        && self
                            .hir
                            .modules
                            .get(&s.module)
                            .map(|m| m.name == "of:core")
                            .unwrap_or(false)
                })
                .map(|(id, _)| *id)
                .expect("of:core::List"),
            vec![k_ty.clone()],
        );
        let keys_mir_ty = self.lower_concrete_type(&keys_ty);
        let keys_local = b.new_local(Some("__keys".to_string()), keys_mir_ty);
        b.push_stmt(Stmt::Assign(
            keys_local,
            AssignValue::Call(Callee::Static(keys_mir_fn), vec![Operand::Copy(it_local)]),
        ));

        // Reuse the list loop machinery but produce Entry<K,V> for the body.
        let entry_id = self
            .entry_struct
            .expect("of:core::Entry must be registered");
        let entry_struct_mir =
            self.get_or_create_struct(entry_id, vec![k_ty.clone(), v_ty.clone()]);

        let list_id = match &keys_ty {
            ResolvedType::Struct(id, _) => *id,
            _ => unreachable!(),
        };
        let list_args = vec![k_ty.clone()];

        let (size_fn, size_extra) = self.resolve_method_with_args(list_id, &list_args, "size");
        let mut size_args = list_args.clone();
        size_args.extend(size_extra);
        let size_mir_fn = self.mono_fn(size_fn, size_args);
        let n_local = b.new_temp(MirType::Primitive(PrimitiveType::Int64));
        b.push_stmt(Stmt::Assign(
            n_local,
            AssignValue::Call(Callee::Static(size_mir_fn), vec![Operand::Copy(keys_local)]),
        ));

        let i_local = b.new_local(Some("__i".to_string()), MirType::Primitive(PrimitiveType::Int64));
        b.push_stmt(Stmt::Assign(
            i_local,
            AssignValue::Use(Operand::Const(MirConst::Int(0, PrimitiveType::Int64))),
        ));

        let head_bb = b.new_block();
        let body_bb = b.new_block();
        let cont_bb = b.new_block();
        let exit_bb = b.new_block();
        b.terminate(Terminator::Goto(head_bb));

        b.switch_to(head_bb);
        let done = b.emit(
            AssignValue::Bin(BinOp::Ge, Operand::Copy(i_local), Operand::Copy(n_local)),
            MirType::Primitive(PrimitiveType::Bool),
        );
        b.terminate(Terminator::CondBr(done, exit_bb, body_bb));

        b.switch_to(body_bb);
        // k = keys.get(i) as K
        let (kget_fn, kget_extra) = self.resolve_method_with_args(list_id, &list_args, "get");
        let mut kget_call_args = list_args.clone();
        kget_call_args.extend(kget_extra);
        let kget_mir_fn = self.mono_fn(kget_fn, kget_call_args);
        let k_nullable = self.maybe_nullable_for(&k_ty);
        let k_opt = b.new_temp(k_nullable.clone());
        b.push_stmt(Stmt::Assign(
            k_opt,
            AssignValue::Call(
                Callee::Static(kget_mir_fn),
                vec![Operand::Copy(keys_local), Operand::Copy(i_local)],
            ),
        ));
        let k_mir_ty = self.lower_concrete_type(&k_ty);
        let k_local = b.new_temp(k_mir_ty.clone());
        match &k_nullable {
            MirType::NullableRef(_) => {
                b.push_stmt(Stmt::Assign(k_local, AssignValue::Use(Operand::Copy(k_opt))));
            }
            MirType::Union(uid) => {
                b.push_stmt(Stmt::Assign(
                    k_local,
                    AssignValue::UnionPayload(Operand::Copy(k_opt), *uid),
                ));
            }
            _ => unreachable!(),
        }

        // v = map.get(k) as V
        let (vget_fn, vget_extra) = self.resolve_method_with_args(map_id, &map_args, "get");
        let mut vget_call_args = map_args.clone();
        vget_call_args.extend(vget_extra);
        let vget_mir_fn = self.mono_fn(vget_fn, vget_call_args);
        let v_nullable = self.maybe_nullable_for(&v_ty);
        let v_opt = b.new_temp(v_nullable.clone());
        b.push_stmt(Stmt::Assign(
            v_opt,
            AssignValue::Call(
                Callee::Static(vget_mir_fn),
                vec![Operand::Copy(it_local), Operand::Copy(k_local)],
            ),
        ));
        let v_mir_ty = self.lower_concrete_type(&v_ty);
        let v_local = b.new_temp(v_mir_ty.clone());
        match &v_nullable {
            MirType::NullableRef(_) => {
                b.push_stmt(Stmt::Assign(v_local, AssignValue::Use(Operand::Copy(v_opt))));
            }
            MirType::Union(uid) => {
                b.push_stmt(Stmt::Assign(
                    v_local,
                    AssignValue::UnionPayload(Operand::Copy(v_opt), *uid),
                ));
            }
            _ => unreachable!(),
        }

        // entry = Entry { key: k, value: v }
        let entry_local = b.new_local(
            Some(var_name.to_string()),
            MirType::ManagedRef(entry_struct_mir),
        );
        b.push_stmt(Stmt::Assign(
            entry_local,
            AssignValue::AllocStruct(
                entry_struct_mir,
                vec![Operand::Copy(k_local), Operand::Copy(v_local)],
            ),
        ));
        b.bind(var_name.to_string(), entry_local);

        b.loop_stack.push((cont_bb, exit_bb));
        let _ = self.lower_block(body, subst, b);
        b.loop_stack.pop();
        b.terminate_if_open(Terminator::Goto(cont_bb));

        b.switch_to(cont_bb);
        let next_i = b.emit(
            AssignValue::Bin(
                BinOp::Add,
                Operand::Copy(i_local),
                Operand::Const(MirConst::Int(1, PrimitiveType::Int64)),
            ),
            MirType::Primitive(PrimitiveType::Int64),
        );
        b.push_stmt(Stmt::Assign(i_local, AssignValue::Use(next_i)));
        b.terminate(Terminator::Goto(head_bb));

        b.switch_to(exit_bb);
    }

    fn lower_for_iterator(
        &mut self,
        var_name: &str,
        iter_e: &TypedExpr,
        iter_ty: &ResolvedType,
        body: &HirBlock,
        subst: &Subst,
        b: &mut FnBuilder,
    ) {
        // Element type from Iterator<T> on iter_ty.
        let elem_ty = self.iterator_element_for(iter_ty);
        let iter_iface_hir = self
            .iterator_interface
            .expect("of:core::Iterator must be registered");
        let iface_mir = self.get_or_create_interface(iter_iface_hir, vec![elem_ty.clone()]);

        let iter_op = self.lower_expr(iter_e, subst, b);
        let it_mir = self.lower_concrete_type(iter_ty);
        let it_local = b.new_local(Some("__it".to_string()), it_mir);
        b.push_stmt(Stmt::Assign(it_local, AssignValue::Use(iter_op)));

        let slot = match &self.mir.types[&iface_mir] {
            MirTypeDef::Interface { method_slots, .. } => method_slots
                .iter()
                .position(|s| s.name == "next")
                .expect("Iterator<T> must have a `next` slot") as u32,
            _ => unreachable!(),
        };

        let head_bb = b.new_block();
        let body_setup_bb = b.new_block();
        let body_bb = b.new_block();
        let exit_bb = b.new_block();
        b.terminate(Terminator::Goto(head_bb));

        b.switch_to(head_bb);
        let next_ret_ty = self.maybe_nullable_for(&elem_ty);
        let n_local = b.new_temp(next_ret_ty.clone());
        b.push_stmt(Stmt::Assign(
            n_local,
            AssignValue::Call(
                Callee::Virtual(Operand::Copy(it_local), iface_mir, slot),
                vec![Operand::Copy(it_local)],
            ),
        ));
        let is_null = match &next_ret_ty {
            MirType::NullableRef(_) => b.emit(
                AssignValue::Bin(
                    BinOp::Eq,
                    Operand::Copy(n_local),
                    Operand::Const(MirConst::Null),
                ),
                MirType::Primitive(PrimitiveType::Bool),
            ),
            MirType::Union(_) => {
                let tag = b.emit(
                    AssignValue::UnionTag(Operand::Copy(n_local)),
                    MirType::Primitive(PrimitiveType::Uint16),
                );
                b.emit(
                    AssignValue::Bin(
                        BinOp::Eq,
                        tag,
                        Operand::Const(MirConst::Int(NULL_TAG as i64, PrimitiveType::Uint16)),
                    ),
                    MirType::Primitive(PrimitiveType::Bool),
                )
            }
            other => panic!("Iterator.next return shape: {:?}", other),
        };
        b.terminate(Terminator::CondBr(is_null, exit_bb, body_setup_bb));

        b.switch_to(body_setup_bb);
        let elem_mir = self.lower_concrete_type(&elem_ty);
        let x_local = b.new_local(Some(var_name.to_string()), elem_mir);
        match &next_ret_ty {
            MirType::NullableRef(_) => {
                b.push_stmt(Stmt::Assign(x_local, AssignValue::Use(Operand::Copy(n_local))));
            }
            MirType::Union(uid) => {
                b.push_stmt(Stmt::Assign(
                    x_local,
                    AssignValue::UnionPayload(Operand::Copy(n_local), *uid),
                ));
            }
            _ => unreachable!(),
        }
        b.terminate(Terminator::Goto(body_bb));

        b.switch_to(body_bb);
        b.bind(var_name.to_string(), x_local);
        b.loop_stack.push((head_bb, exit_bb));
        let _ = self.lower_block(body, subst, b);
        b.loop_stack.pop();
        b.terminate_if_open(Terminator::Goto(head_bb));

        b.switch_to(exit_bb);
    }

    fn iterator_element_for(&self, iter_ty: &ResolvedType) -> ResolvedType {
        match iter_ty {
            ResolvedType::Interface(id, args)
                if Some(*id) == self.iterator_interface && args.len() == 1 =>
            {
                args[0].clone()
            }
            ResolvedType::Struct(id, args) => {
                let s = &self.hir.structs[id];
                let subst_map: std::collections::HashMap<_, _> =
                    s.type_params.iter().copied().zip(args.iter().cloned()).collect();
                for (iface, iface_args) in &s.implements {
                    if Some(*iface) == self.iterator_interface && iface_args.len() == 1 {
                        return substitute_simple(&iface_args[0], &subst_map);
                    }
                }
                panic!("struct {} does not implement Iterator<T>", s.name);
            }
            other => panic!("for-in target: {:?}", other),
        }
    }

    /// Build the nullable wrapper around `elem`: `NullableRef(T)` when T is
    /// a managed ref, otherwise a tagged union `[elem, null]`.
    fn maybe_nullable_for(&mut self, elem: &ResolvedType) -> MirType {
        let variants = vec![elem.clone(), ResolvedType::Null];
        if let Some(inner_mid) = self.try_nullable_ref(&variants) {
            return MirType::NullableRef(inner_mid);
        }
        let mid = self.get_or_create_union(variants);
        MirType::Union(mid)
    }

    fn lower_while(&mut self, cond: &TypedExpr, body: &HirBlock, subst: &Subst, b: &mut FnBuilder) {
        let head_bb = b.new_block();
        let body_bb = b.new_block();
        let exit_bb = b.new_block();

        // Jump from current block into head.
        b.terminate(Terminator::Goto(head_bb));

        // head: evaluate cond, branch.
        b.switch_to(head_bb);
        let cond_op = self.lower_expr(cond, subst, b);
        b.terminate(Terminator::CondBr(cond_op, body_bb, exit_bb));

        // body: lower with loop frame on the stack.
        b.switch_to(body_bb);
        b.loop_stack.push((head_bb, exit_bb));
        let _ = self.lower_block(body, subst, b);
        b.loop_stack.pop();
        // Body falls through → jump to head, unless body terminated already.
        b.terminate_if_open(Terminator::Goto(head_bb));

        // Continue lowering after the loop.
        b.switch_to(exit_bb);
    }
}

fn substitute_simple(
    ty: &ResolvedType,
    subst: &std::collections::HashMap<crate::hir::TypeParamId, ResolvedType>,
) -> ResolvedType {
    use ResolvedType::*;
    match ty {
        Primitive(p) => Primitive(p.clone()),
        Null => Null,
        TypeParam(id) => subst.get(id).cloned().unwrap_or(TypeParam(*id)),
        Struct(id, args) => Struct(
            *id,
            args.iter().map(|a| substitute_simple(a, subst)).collect(),
        ),
        Interface(id, args) => Interface(
            *id,
            args.iter().map(|a| substitute_simple(a, subst)).collect(),
        ),
        Union(vs) => Union(vs.iter().map(|v| substitute_simple(v, subst)).collect()),
        Function(args, ret) => Function(
            args.iter().map(|a| substitute_simple(a, subst)).collect(),
            Box::new(substitute_simple(ret, subst)),
        ),
    }
}
