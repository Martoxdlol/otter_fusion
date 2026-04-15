use crate::ast;
use crate::hir::{self, ExprKind, FnId, HirBlock, HirLiteral, HirStatement, ModuleId, ResolvedType, TypeId, TypedExpr};

use super::errors::{ErrorKind, ValidationError};
use super::resolve::substitute_type_params;
use super::scope::LocalScope;
use super::Validator;

impl Validator {
    /// Pass 5: Type-check all function bodies
    pub(crate) fn pass_typecheck_bodies(&mut self) {
        let fn_ids: Vec<FnId> = self.ast_fn_bodies.keys().copied().collect();

        for fn_id in fn_ids {
            self.typecheck_function(fn_id);
        }
    }

    fn typecheck_function(&mut self, fn_id: FnId) {
        let hir_fn = match self.hir.functions.get(&fn_id) {
            Some(f) => f.clone(),
            None => return,
        };

        let ast_body = match self.ast_fn_bodies.get(&fn_id) {
            Some(b) => b.clone(),
            None => return,
        };

        let module_id = hir_fn.module;
        let module_name = self.module_name(module_id);

        // Set up type param scope
        self.type_param_scope.clear();

        // If method, bring owner's type params into scope
        if let Some(owner_id) = hir_fn.owner {
            let owner_tps = if let Some(s) = self.hir.structs.get(&owner_id) {
                s.type_params.clone()
            } else if let Some(i) = self.hir.interfaces.get(&owner_id) {
                i.type_params.clone()
            } else {
                vec![]
            };
            for &tp_id in &owner_tps {
                if let Some(tp_info) = self.hir.type_params.get(&tp_id) {
                    self.type_param_scope.insert(tp_info.name.clone(), tp_id);
                }
            }
        }

        // Bring function's own type params into scope
        for &tp_id in &hir_fn.type_params {
            if let Some(tp_info) = self.hir.type_params.get(&tp_id) {
                self.type_param_scope.insert(tp_info.name.clone(), tp_id);
            }
        }

        // Set up local scope with parameters
        let mut scope = LocalScope::new();

        // Bind self if method
        if hir_fn.has_self {
            if let Some(owner_id) = hir_fn.owner {
                let self_type = self.make_self_type(owner_id);
                scope.define("self".to_string(), self_type);
            }
        }

        // Bind parameters
        for param in &hir_fn.params {
            scope.define(param.name.clone(), param.ty.clone());
        }

        let expected_return = hir_fn.return_type.clone();
        let context = format!("in function '{}'", hir_fn.name);

        // Type-check the block
        match self.typecheck_block(&ast_body, &mut scope, module_id, &expected_return, &context, &module_name) {
            Ok(hir_block) => {
                // Check return type
                if let Some(ref ret_expr) = hir_block.returns {
                    if !self.types_compatible(&expected_return, &ret_expr.ty) {
                        self.errors.push(ValidationError {
                            kind: ErrorKind::ReturnTypeMismatch {
                                expected: self.format_type(&expected_return),
                                found: self.format_type(&ret_expr.ty),
                            },
                            module: module_name.clone(),
                            context: Some(context.clone()),
                        });
                    }
                }
                self.hir.functions.get_mut(&fn_id).unwrap().body = Some(hir_block);
            }
            Err(e) => {
                self.errors.extend(e);
            }
        }

        self.type_param_scope.clear();
    }

    fn make_self_type(&self, owner_id: TypeId) -> ResolvedType {
        if let Some(s) = self.hir.structs.get(&owner_id) {
            let args: Vec<_> = s.type_params.iter().map(|&tp| ResolvedType::TypeParam(tp)).collect();
            ResolvedType::Struct(owner_id, args)
        } else if let Some(i) = self.hir.interfaces.get(&owner_id) {
            let args: Vec<_> = i.type_params.iter().map(|&tp| ResolvedType::TypeParam(tp)).collect();
            ResolvedType::Interface(owner_id, args)
        } else {
            ResolvedType::Null
        }
    }

    fn typecheck_block(
        &mut self,
        block: &ast::Block,
        scope: &mut LocalScope,
        module_id: ModuleId,
        expected_return: &ResolvedType,
        context: &str,
        module_name: &str,
    ) -> Result<HirBlock, Vec<ValidationError>> {
        let mut errors = Vec::new();
        let mut hir_stmts = Vec::new();

        scope.push();

        for stmt in &block.statements {
            match self.typecheck_statement(stmt, scope, module_id, expected_return, context, module_name) {
                Ok(hir_stmt) => hir_stmts.push(hir_stmt),
                Err(mut e) => errors.append(&mut e),
            }
        }

        let returns = if let Some(ret_expr) = &block.returns {
            match self.typecheck_expr(ret_expr, scope, module_id, context, module_name) {
                Ok(typed) => Some(typed),
                Err(mut e) => {
                    errors.append(&mut e);
                    None
                }
            }
        } else {
            None
        };

        scope.pop();

        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(HirBlock {
            statements: hir_stmts,
            returns,
        })
    }

    fn typecheck_statement(
        &mut self,
        stmt: &ast::Statement,
        scope: &mut LocalScope,
        module_id: ModuleId,
        expected_return: &ResolvedType,
        context: &str,
        module_name: &str,
    ) -> Result<HirStatement, Vec<ValidationError>> {
        match stmt {
            ast::Statement::VarDecl(name, type_ann, init) => {
                let resolved_ann = if let Some(ann) = type_ann {
                    match self.resolve_type(module_id, ann) {
                        Ok(t) => Some(t),
                        Err(e) => return Err(vec![e]),
                    }
                } else {
                    None
                };

                let typed_init = if let Some(init_expr) = init {
                    match self.typecheck_expr(init_expr, scope, module_id, context, module_name) {
                        Ok(t) => Some(t),
                        Err(e) => return Err(e),
                    }
                } else {
                    None
                };

                let var_type = match (&resolved_ann, &typed_init) {
                    (Some(ann), Some(init_typed)) => {
                        if !self.types_compatible(ann, &init_typed.ty) {
                            return Err(vec![ValidationError {
                                kind: ErrorKind::TypeMismatch {
                                    expected: self.format_type(ann),
                                    found: self.format_type(&init_typed.ty),
                                },
                                module: module_name.to_string(),
                                context: Some(format!("{context}, var '{name}'")),
                            }]);
                        }
                        ann.clone()
                    }
                    (Some(ann), None) => ann.clone(),
                    (None, Some(init_typed)) => init_typed.ty.clone(),
                    (None, None) => {
                        return Err(vec![ValidationError {
                            kind: ErrorKind::CannotInferType(name.clone()),
                            module: module_name.to_string(),
                            context: Some(context.to_string()),
                        }]);
                    }
                };

                scope.define(name.clone(), var_type.clone());
                Ok(HirStatement::VarDecl(name.clone(), var_type, typed_init))
            }

            ast::Statement::Return(expr) => {
                let typed = if let Some(e) = expr {
                    let t = self.typecheck_expr(e, scope, module_id, context, module_name)?;
                    if !self.types_compatible(expected_return, &t.ty) {
                        return Err(vec![ValidationError {
                            kind: ErrorKind::ReturnTypeMismatch {
                                expected: self.format_type(expected_return),
                                found: self.format_type(&t.ty),
                            },
                            module: module_name.to_string(),
                            context: Some(context.to_string()),
                        }]);
                    }
                    Some(t)
                } else {
                    None
                };
                Ok(HirStatement::Return(typed))
            }

            ast::Statement::Expr(expr) => {
                let typed = self.typecheck_expr(expr, scope, module_id, context, module_name)?;
                Ok(HirStatement::Expr(typed))
            }

            ast::Statement::While(cond, body) => {
                let typed_cond = self.typecheck_expr(cond, scope, module_id, context, module_name)?;
                if !self.types_compatible(&ResolvedType::Primitive(hir::PrimitiveType::Bool), &typed_cond.ty) {
                    return Err(vec![ValidationError {
                        kind: ErrorKind::ConditionNotBool(self.format_type(&typed_cond.ty)),
                        module: module_name.to_string(),
                        context: Some(context.to_string()),
                    }]);
                }
                scope.push_loop();
                let hir_body = self.typecheck_block(body, scope, module_id, expected_return, context, module_name)?;
                scope.pop_loop();
                Ok(HirStatement::While(typed_cond, hir_body))
            }

            ast::Statement::For(var_name, iterable, body) => {
                let typed_iter = self.typecheck_expr(iterable, scope, module_id, context, module_name)?;

                // Determine element type: iterable must be List<T>
                // Unfold aliases to find the underlying type
                let iter_ty = self.shallow_unfold_alias(module_id, &typed_iter.ty);
                let elem_type = match &iter_ty {
                    ResolvedType::Struct(id, args) if Some(*id) == self.list_type_id => {
                        if args.is_empty() {
                            ResolvedType::Null
                        } else {
                            args[0].clone()
                        }
                    }
                    other => {
                        return Err(vec![ValidationError {
                            kind: ErrorKind::NotIterable(self.format_type(other)),
                            module: module_name.to_string(),
                            context: Some(context.to_string()),
                        }]);
                    }
                };

                scope.push_loop();
                scope.define(var_name.clone(), elem_type);
                let hir_body = self.typecheck_block(body, scope, module_id, expected_return, context, module_name)?;
                scope.pop_loop();
                Ok(HirStatement::For(var_name.clone(), typed_iter, hir_body))
            }

            ast::Statement::Break => {
                if !scope.is_in_loop() {
                    return Err(vec![ValidationError {
                        kind: ErrorKind::BreakOutsideLoop,
                        module: module_name.to_string(),
                        context: Some(context.to_string()),
                    }]);
                }
                Ok(HirStatement::Break)
            }

            ast::Statement::Continue => {
                if !scope.is_in_loop() {
                    return Err(vec![ValidationError {
                        kind: ErrorKind::ContinueOutsideLoop,
                        module: module_name.to_string(),
                        context: Some(context.to_string()),
                    }]);
                }
                Ok(HirStatement::Continue)
            }
        }
    }

    fn typecheck_expr(
        &mut self,
        expr: &ast::Expr,
        scope: &mut LocalScope,
        module_id: ModuleId,
        context: &str,
        module_name: &str,
    ) -> Result<TypedExpr, Vec<ValidationError>> {
        match expr {
            ast::Expr::Literal(lit) => self.typecheck_literal(lit, module_name, context),

            ast::Expr::Variable(name) => {
                // Check local scope
                if let Some(ty) = scope.lookup(name) {
                    return Ok(TypedExpr {
                        kind: ExprKind::Variable(name.clone()),
                        ty: ty.clone(),
                    });
                }

                // Check module-level functions
                let visible = self.visible_names.get(&module_id).cloned().unwrap_or_default();
                if let Some(&fn_id) = visible.functions.get(name.as_str()) {
                    let hir_fn = &self.hir.functions[&fn_id];
                    let param_types: Vec<_> = hir_fn.params.iter().map(|p| p.ty.clone()).collect();
                    let ret_type = hir_fn.return_type.clone();
                    return Ok(TypedExpr {
                        kind: ExprKind::Variable(name.clone()),
                        ty: ResolvedType::Function(param_types, Box::new(ret_type)),
                    });
                }

                Err(vec![ValidationError {
                    kind: ErrorKind::UndefinedVariable(name.clone()),
                    module: module_name.to_string(),
                    context: Some(context.to_string()),
                }])
            }

            ast::Expr::BinaryOp(lhs, op, rhs) => {
                let typed_lhs = self.typecheck_expr(lhs, scope, module_id, context, module_name)?;
                let typed_rhs = self.typecheck_expr(rhs, scope, module_id, context, module_name)?;

                let hir_op = convert_binary_op(op);
                let result_type = self.check_binary_op(&hir_op, &typed_lhs.ty, &typed_rhs.ty, module_name, context)?;

                Ok(TypedExpr {
                    kind: ExprKind::BinaryOp(Box::new(typed_lhs), hir_op, Box::new(typed_rhs)),
                    ty: result_type,
                })
            }

            ast::Expr::UnaryOp(op, operand) => {
                let typed_operand = self.typecheck_expr(operand, scope, module_id, context, module_name)?;
                let hir_op = convert_unary_op(op);
                let result_type = self.check_unary_op(&hir_op, &typed_operand.ty, module_name, context)?;

                Ok(TypedExpr {
                    kind: ExprKind::UnaryOp(hir_op, Box::new(typed_operand)),
                    ty: result_type,
                })
            }

            ast::Expr::Call(callee, type_args, args) => {
                let typed_callee = self.typecheck_expr(callee, scope, module_id, context, module_name)?;

                // Resolve type arguments
                let resolved_type_args: Vec<ResolvedType> = type_args
                    .iter()
                    .map(|ta| self.resolve_type(module_id, ta).map_err(|e| vec![e]))
                    .collect::<Result<Vec<_>, _>>()?;

                // Type-check arguments
                let mut typed_args = Vec::new();
                for arg in args {
                    typed_args.push(self.typecheck_expr(arg, scope, module_id, context, module_name)?);
                }

                // Determine return type — unfold aliases to find Function type
                let callee_ty = self.shallow_unfold_alias(module_id, &typed_callee.ty);
                match &callee_ty {
                    ResolvedType::Function(param_types, ret_type) => {
                        if param_types.len() != typed_args.len() {
                            return Err(vec![ValidationError {
                                kind: ErrorKind::WrongArgCount {
                                    expected: param_types.len(),
                                    found: typed_args.len(),
                                },
                                module: module_name.to_string(),
                                context: Some(context.to_string()),
                            }]);
                        }
                        for (i, (expected, actual)) in param_types.iter().zip(&typed_args).enumerate() {
                            if !self.types_compatible(expected, &actual.ty) {
                                return Err(vec![ValidationError {
                                    kind: ErrorKind::TypeMismatch {
                                        expected: self.format_type(expected),
                                        found: self.format_type(&actual.ty),
                                    },
                                    module: module_name.to_string(),
                                    context: Some(format!("{context}, argument {i}")),
                                }]);
                            }
                        }
                        Ok(TypedExpr {
                            kind: ExprKind::Call(
                                Box::new(typed_callee),
                                resolved_type_args,
                                typed_args,
                            ),
                            ty: *ret_type.clone(),
                        })
                    }
                    other => Err(vec![ValidationError {
                        kind: ErrorKind::NotCallable(self.format_type(other)),
                        module: module_name.to_string(),
                        context: Some(context.to_string()),
                    }]),
                }
            }

            ast::Expr::Member(obj, member_name) => {
                let typed_obj = self.typecheck_expr(obj, scope, module_id, context, module_name)?;
                self.typecheck_member_access(typed_obj, member_name, module_id, module_name, context)
            }

            ast::Expr::If(cond, then_block, else_block) => {
                let typed_cond = self.typecheck_expr(cond, scope, module_id, context, module_name)?;
                if !self.types_compatible(&ResolvedType::Primitive(hir::PrimitiveType::Bool), &typed_cond.ty) {
                    return Err(vec![ValidationError {
                        kind: ErrorKind::ConditionNotBool(self.format_type(&typed_cond.ty)),
                        module: module_name.to_string(),
                        context: Some(context.to_string()),
                    }]);
                }

                let expected_ret = ResolvedType::Null; // if-as-expression: we infer from branches
                let typed_then = self.typecheck_block(then_block, scope, module_id, &expected_ret, context, module_name)?;

                let then_type = typed_then
                    .returns
                    .as_ref()
                    .map(|e| e.ty.clone())
                    .unwrap_or(ResolvedType::Null);

                let (typed_else, result_type) = if let Some(else_blk) = else_block {
                    let typed_else_block = self.typecheck_block(else_blk, scope, module_id, &expected_ret, context, module_name)?;
                    let else_type = typed_else_block
                        .returns
                        .as_ref()
                        .map(|e| e.ty.clone())
                        .unwrap_or(ResolvedType::Null);

                    let result = if self.types_compatible(&then_type, &else_type) {
                        then_type
                    } else if self.types_compatible(&else_type, &then_type) {
                        else_type
                    } else {
                        // Create union
                        let mut variants = Vec::new();
                        match &then_type {
                            ResolvedType::Union(v) => variants.extend(v.clone()),
                            other => variants.push(other.clone()),
                        }
                        match &else_type {
                            ResolvedType::Union(v) => {
                                for vv in v {
                                    if !variants.contains(vv) {
                                        variants.push(vv.clone());
                                    }
                                }
                            }
                            other => {
                                if !variants.contains(other) {
                                    variants.push(other.clone());
                                }
                            }
                        }
                        if variants.len() == 1 {
                            variants.into_iter().next().unwrap()
                        } else {
                            ResolvedType::Union(variants)
                        }
                    };
                    (Some(Box::new(typed_else_block)), result)
                } else {
                    (None, then_type)
                };

                Ok(TypedExpr {
                    kind: ExprKind::If(
                        Box::new(typed_cond),
                        Box::new(typed_then),
                        typed_else,
                    ),
                    ty: result_type,
                })
            }

            ast::Expr::StructInit(type_expr, fields) => {
                let resolved = self.resolve_type(module_id, type_expr).map_err(|e| vec![e])?;
                let (struct_id, type_args) = match &resolved {
                    ResolvedType::Struct(id, args) => (*id, args.clone()),
                    _ => {
                        return Err(vec![ValidationError {
                            kind: ErrorKind::UndefinedType(format!("{type_expr:?}")),
                            module: module_name.to_string(),
                            context: Some(format!("{context}, struct init")),
                        }]);
                    }
                };

                let hir_struct = match self.hir.structs.get(&struct_id) {
                    Some(s) => s.clone(),
                    None => {
                        return Err(vec![ValidationError {
                            kind: ErrorKind::UndefinedType(format!("{type_expr:?}")),
                            module: module_name.to_string(),
                            context: Some(context.to_string()),
                        }]);
                    }
                };

                let struct_tp_ids = hir_struct.type_params.clone();

                // Type-check each field
                let mut typed_fields = Vec::new();
                let mut provided_names: Vec<String> = Vec::new();

                for (field_name, field_expr) in fields {
                    let typed_val = self.typecheck_expr(field_expr, scope, module_id, context, module_name)?;
                    provided_names.push(field_name.clone());

                    // Find the field in the struct
                    let expected_field = hir_struct.fields.iter().find(|f| f.name == *field_name);
                    match expected_field {
                        Some(ef) => {
                            let substituted = substitute_type_params(&ef.ty, &struct_tp_ids, &type_args);
                            if !self.types_compatible(&substituted, &typed_val.ty) {
                                return Err(vec![ValidationError {
                                    kind: ErrorKind::TypeMismatch {
                                        expected: self.format_type(&substituted),
                                        found: self.format_type(&typed_val.ty),
                                    },
                                    module: module_name.to_string(),
                                    context: Some(format!("{context}, field '{field_name}'")),
                                }]);
                            }
                        }
                        None => {
                            return Err(vec![ValidationError {
                                kind: ErrorKind::ExtraField {
                                    struct_name: hir_struct.name.clone(),
                                    field: field_name.clone(),
                                },
                                module: module_name.to_string(),
                                context: Some(context.to_string()),
                            }]);
                        }
                    }
                    typed_fields.push((field_name.clone(), typed_val));
                }

                // Check for missing required fields
                for req_field in &hir_struct.fields {
                    if !provided_names.contains(&req_field.name) {
                        return Err(vec![ValidationError {
                            kind: ErrorKind::MissingField {
                                struct_name: hir_struct.name.clone(),
                                field: req_field.name.clone(),
                            },
                            module: module_name.to_string(),
                            context: Some(context.to_string()),
                        }]);
                    }
                }

                Ok(TypedExpr {
                    kind: ExprKind::StructInit(struct_id, type_args, typed_fields),
                    ty: resolved,
                })
            }

            ast::Expr::As(expr, target_type) => {
                let typed = self.typecheck_expr(expr, scope, module_id, context, module_name)?;
                let resolved_target = self.resolve_type(module_id, target_type).map_err(|e| vec![e])?;
                Ok(TypedExpr {
                    kind: ExprKind::As(Box::new(typed), resolved_target.clone()),
                    ty: resolved_target,
                })
            }

            ast::Expr::Is(expr, check_type) => {
                let typed = self.typecheck_expr(expr, scope, module_id, context, module_name)?;
                let resolved_check = self.resolve_type(module_id, check_type).map_err(|e| vec![e])?;
                Ok(TypedExpr {
                    kind: ExprKind::Is(Box::new(typed), resolved_check),
                    ty: ResolvedType::Primitive(hir::PrimitiveType::Bool),
                })
            }

            ast::Expr::LiteralList(elements) => {
                let mut typed_elems = Vec::new();
                let mut elem_types: Vec<ResolvedType> = Vec::new();

                for elem in elements {
                    let typed = self.typecheck_expr(elem, scope, module_id, context, module_name)?;
                    if !elem_types.contains(&typed.ty) {
                        elem_types.push(typed.ty.clone());
                    }
                    typed_elems.push(typed);
                }

                let elem_type = if elem_types.is_empty() {
                    ResolvedType::Null
                } else if elem_types.len() == 1 {
                    elem_types.into_iter().next().unwrap()
                } else {
                    ResolvedType::Union(elem_types)
                };

                let list_id = self.list_type_id.unwrap();
                let result_type = ResolvedType::Struct(list_id, vec![elem_type]);

                Ok(TypedExpr {
                    kind: ExprKind::LiteralList(typed_elems),
                    ty: result_type,
                })
            }

            ast::Expr::LiteralMap(entries) => {
                let mut typed_entries = Vec::new();
                let mut key_types: Vec<ResolvedType> = Vec::new();
                let mut val_types: Vec<ResolvedType> = Vec::new();

                for (k, v) in entries {
                    let typed_k = self.typecheck_expr(k, scope, module_id, context, module_name)?;
                    let typed_v = self.typecheck_expr(v, scope, module_id, context, module_name)?;
                    if !key_types.contains(&typed_k.ty) {
                        key_types.push(typed_k.ty.clone());
                    }
                    if !val_types.contains(&typed_v.ty) {
                        val_types.push(typed_v.ty.clone());
                    }
                    typed_entries.push((typed_k, typed_v));
                }

                let key_type = if key_types.is_empty() {
                    ResolvedType::Null
                } else if key_types.len() == 1 {
                    key_types.into_iter().next().unwrap()
                } else {
                    ResolvedType::Union(key_types)
                };

                let val_type = if val_types.is_empty() {
                    ResolvedType::Null
                } else if val_types.len() == 1 {
                    val_types.into_iter().next().unwrap()
                } else {
                    ResolvedType::Union(val_types)
                };

                let map_id = self.map_type_id.unwrap();
                let result_type = ResolvedType::Struct(map_id, vec![key_type, val_type]);

                Ok(TypedExpr {
                    kind: ExprKind::LiteralMap(typed_entries),
                    ty: result_type,
                })
            }

            ast::Expr::Block(block) => {
                let expected_ret = ResolvedType::Null;
                let hir_block = self.typecheck_block(block, scope, module_id, &expected_ret, context, module_name)?;
                let block_type = hir_block
                    .returns
                    .as_ref()
                    .map(|e| e.ty.clone())
                    .unwrap_or(ResolvedType::Null);

                Ok(TypedExpr {
                    kind: ExprKind::Block(Box::new(hir_block)),
                    ty: block_type,
                })
            }

            ast::Expr::FunctionLiteral(generics, params, ret_type_expr, body) => {
                let saved_tp_scope = self.type_param_scope.clone();

                // Allocate type params
                let tp_ids = self.allocate_type_params(generics);
                for (i, gp) in generics.iter().enumerate() {
                    self.type_param_scope.insert(gp.name.clone(), tp_ids[i]);
                }

                // Resolve param types
                let mut hir_params = Vec::new();
                let mut param_types = Vec::new();
                for param in params {
                    let resolved = self.resolve_type(module_id, &param.ty).map_err(|e| vec![e])?;
                    param_types.push(resolved.clone());
                    hir_params.push(hir::HirParam {
                        name: param.name.clone(),
                        ty: resolved,
                    });
                }

                // Resolve return type
                let resolved_ret = self.resolve_type(module_id, ret_type_expr).map_err(|e| vec![e])?;

                // Type-check body
                let mut fn_scope = LocalScope::new();
                for param in &hir_params {
                    fn_scope.define(param.name.clone(), param.ty.clone());
                }

                let hir_body = self.typecheck_block(body, &mut fn_scope, module_id, &resolved_ret, context, module_name)?;

                // Capture analysis: find variables referenced from enclosing scope
                let captures = self.collect_captures(body, scope);

                self.type_param_scope = saved_tp_scope;

                let fn_type = ResolvedType::Function(param_types, Box::new(resolved_ret));

                Ok(TypedExpr {
                    kind: ExprKind::FunctionLiteral(tp_ids, hir_params, captures, Box::new(hir_body)),
                    ty: fn_type,
                })
            }
        }
    }

    fn typecheck_literal(
        &self,
        lit: &ast::Literal,
        module_name: &str,
        context: &str,
    ) -> Result<TypedExpr, Vec<ValidationError>> {
        match lit {
            ast::Literal::Int(s) => {
                let val: i64 = s.parse().map_err(|_| {
                    vec![ValidationError {
                        kind: ErrorKind::InvalidIntLiteral(s.clone()),
                        module: module_name.to_string(),
                        context: Some(context.to_string()),
                    }]
                })?;
                Ok(TypedExpr {
                    kind: ExprKind::Literal(HirLiteral::Int(val)),
                    ty: ResolvedType::Primitive(hir::PrimitiveType::Int64),
                })
            }
            ast::Literal::Float(s) => {
                let val: f64 = s.parse().map_err(|_| {
                    vec![ValidationError {
                        kind: ErrorKind::InvalidFloatLiteral(s.clone()),
                        module: module_name.to_string(),
                        context: Some(context.to_string()),
                    }]
                })?;
                Ok(TypedExpr {
                    kind: ExprKind::Literal(HirLiteral::Float(val)),
                    ty: ResolvedType::Primitive(hir::PrimitiveType::Float64),
                })
            }
            ast::Literal::String(s) => Ok(TypedExpr {
                kind: ExprKind::Literal(HirLiteral::String(s.clone())),
                ty: ResolvedType::Primitive(hir::PrimitiveType::String),
            }),
            ast::Literal::Char(c) => Ok(TypedExpr {
                kind: ExprKind::Literal(HirLiteral::Char(*c)),
                ty: ResolvedType::Primitive(hir::PrimitiveType::Char),
            }),
            ast::Literal::Bool(b) => Ok(TypedExpr {
                kind: ExprKind::Literal(HirLiteral::Bool(*b)),
                ty: ResolvedType::Primitive(hir::PrimitiveType::Bool),
            }),
            ast::Literal::Null => Ok(TypedExpr {
                kind: ExprKind::Literal(HirLiteral::Null),
                ty: ResolvedType::Null,
            }),
        }
    }

    fn check_binary_op(
        &self,
        op: &hir::BinaryOperator,
        left: &ResolvedType,
        right: &ResolvedType,
        module_name: &str,
        context: &str,
    ) -> Result<ResolvedType, Vec<ValidationError>> {
        let err = |detail: &str| {
            Err(vec![ValidationError {
                kind: ErrorKind::BinaryOpTypeMismatch {
                    op: format!("{op:?}"),
                    left: self.format_type(left),
                    right: self.format_type(right),
                },
                module: module_name.to_string(),
                context: Some(format!("{context}: {detail}")),
            }])
        };

        match op {
            hir::BinaryOperator::Add => {
                // String concat
                if matches!(left, ResolvedType::Primitive(hir::PrimitiveType::String))
                    && matches!(right, ResolvedType::Primitive(hir::PrimitiveType::String))
                {
                    return Ok(ResolvedType::Primitive(hir::PrimitiveType::String));
                }
                // Numeric
                if is_numeric(left) && left == right {
                    return Ok(left.clone());
                }
                err("Add requires same numeric type or string + string")
            }
            hir::BinaryOperator::Sub
            | hir::BinaryOperator::Mul
            | hir::BinaryOperator::Div
            | hir::BinaryOperator::Mod => {
                if is_numeric(left) && left == right {
                    return Ok(left.clone());
                }
                err("arithmetic requires same numeric type")
            }
            hir::BinaryOperator::Lt
            | hir::BinaryOperator::Le
            | hir::BinaryOperator::Gt
            | hir::BinaryOperator::Ge => {
                if is_numeric(left) && left == right {
                    return Ok(ResolvedType::Primitive(hir::PrimitiveType::Bool));
                }
                err("comparison requires same numeric type")
            }
            hir::BinaryOperator::Eq | hir::BinaryOperator::Neq => {
                if self.types_compatible(left, right) || self.types_compatible(right, left) {
                    return Ok(ResolvedType::Primitive(hir::PrimitiveType::Bool));
                }
                err("equality requires compatible types")
            }
            hir::BinaryOperator::And | hir::BinaryOperator::Or => {
                let bool_type = ResolvedType::Primitive(hir::PrimitiveType::Bool);
                if self.types_compatible(&bool_type, left) && self.types_compatible(&bool_type, right) {
                    return Ok(bool_type);
                }
                err("logical operators require bool operands")
            }
        }
    }

    fn check_unary_op(
        &self,
        op: &hir::UnaryOperator,
        operand: &ResolvedType,
        module_name: &str,
        context: &str,
    ) -> Result<ResolvedType, Vec<ValidationError>> {
        match op {
            hir::UnaryOperator::Neg => {
                if is_numeric(operand) {
                    Ok(operand.clone())
                } else {
                    Err(vec![ValidationError {
                        kind: ErrorKind::UnaryOpTypeMismatch {
                            op: "Neg".to_string(),
                            operand: self.format_type(operand),
                        },
                        module: module_name.to_string(),
                        context: Some(context.to_string()),
                    }])
                }
            }
            hir::UnaryOperator::Not => {
                let bool_type = ResolvedType::Primitive(hir::PrimitiveType::Bool);
                if self.types_compatible(&bool_type, operand) {
                    Ok(bool_type)
                } else {
                    Err(vec![ValidationError {
                        kind: ErrorKind::UnaryOpTypeMismatch {
                            op: "Not".to_string(),
                            operand: self.format_type(operand),
                        },
                        module: module_name.to_string(),
                        context: Some(context.to_string()),
                    }])
                }
            }
        }
    }

    /// Check if the concrete type_args of a struct instance match a specialization's type_args.
    fn specialized_args_match(
        &self,
        instance_args: &[ResolvedType],
        spec_args: &[ResolvedType],
    ) -> bool {
        instance_args.len() == spec_args.len()
            && instance_args
                .iter()
                .zip(spec_args)
                .all(|(a, b)| self.types_compatible(a, b) && self.types_compatible(b, a))
    }

    /// Unfold an Alias type one level if it is one, otherwise return as-is.
    /// Used to expose the structural type for pattern matching (member access, for-loop, call).
    fn shallow_unfold_alias(&mut self, module_id: ModuleId, ty: &ResolvedType) -> ResolvedType {
        match ty {
            ResolvedType::Alias(id, args) => {
                self.unfold_alias(module_id, *id, args).unwrap_or_else(|_| ty.clone())
            }
            _ => ty.clone(),
        }
    }

    fn typecheck_member_access(
        &mut self,
        typed_obj: TypedExpr,
        member_name: &str,
        module_id: ModuleId,
        module_name: &str,
        context: &str,
    ) -> Result<TypedExpr, Vec<ValidationError>> {
        // Unfold aliases to find the structural type for member access
        let effective_ty = self.shallow_unfold_alias(module_id, &typed_obj.ty);
        match &effective_ty {
            ResolvedType::Struct(id, type_args) => {
                let hir_struct = match self.hir.structs.get(id) {
                    Some(s) => s.clone(),
                    None => {
                        return Err(vec![ValidationError {
                            kind: ErrorKind::UndefinedMember {
                                ty: self.format_type(&typed_obj.ty),
                                member: member_name.to_string(),
                            },
                            module: module_name.to_string(),
                            context: Some(context.to_string()),
                        }]);
                    }
                };

                let struct_tp_ids = hir_struct.type_params.clone();

                // Check fields
                if let Some(field) = hir_struct.fields.iter().find(|f| f.name == member_name) {
                    let substituted = substitute_type_params(&field.ty, &struct_tp_ids, type_args);
                    return Ok(TypedExpr {
                        kind: ExprKind::Member(Box::new(typed_obj), member_name.to_string()),
                        ty: substituted,
                    });
                }

                // Check specialized methods first (higher priority)
                for spec in &hir_struct.specialized_methods {
                    if self.specialized_args_match(type_args, &spec.type_args) {
                        for &method_id in &spec.methods {
                            if let Some(method) = self.hir.functions.get(&method_id) {
                                if method.name == member_name {
                                    // Specialized methods already have concrete types,
                                    // no substitution needed
                                    let param_types: Vec<_> =
                                        method.params.iter().map(|p| p.ty.clone()).collect();
                                    let ret_type = method.return_type.clone();
                                    return Ok(TypedExpr {
                                        kind: ExprKind::Member(
                                            Box::new(typed_obj),
                                            member_name.to_string(),
                                        ),
                                        ty: ResolvedType::Function(
                                            param_types,
                                            Box::new(ret_type),
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }

                // Check generic methods
                for &method_id in &hir_struct.methods {
                    if let Some(method) = self.hir.functions.get(&method_id) {
                        if method.name == member_name {
                            let param_types: Vec<_> = method
                                .params
                                .iter()
                                .map(|p| substitute_type_params(&p.ty, &struct_tp_ids, type_args))
                                .collect();
                            let ret_type =
                                substitute_type_params(&method.return_type, &struct_tp_ids, type_args);
                            return Ok(TypedExpr {
                                kind: ExprKind::Member(Box::new(typed_obj), member_name.to_string()),
                                ty: ResolvedType::Function(param_types, Box::new(ret_type)),
                            });
                        }
                    }
                }

                Err(vec![ValidationError {
                    kind: ErrorKind::UndefinedMember {
                        ty: hir_struct.name.clone(),
                        member: member_name.to_string(),
                    },
                    module: module_name.to_string(),
                    context: Some(context.to_string()),
                }])
            }

            ResolvedType::Interface(id, type_args) => {
                let hir_iface = match self.hir.interfaces.get(id) {
                    Some(i) => i.clone(),
                    None => {
                        return Err(vec![ValidationError {
                            kind: ErrorKind::UndefinedMember {
                                ty: self.format_type(&typed_obj.ty),
                                member: member_name.to_string(),
                            },
                            module: module_name.to_string(),
                            context: Some(context.to_string()),
                        }]);
                    }
                };

                let iface_tp_ids = hir_iface.type_params.clone();

                if let Some(field) = hir_iface.fields.iter().find(|f| f.name == member_name) {
                    let substituted = substitute_type_params(&field.ty, &iface_tp_ids, type_args);
                    return Ok(TypedExpr {
                        kind: ExprKind::Member(Box::new(typed_obj), member_name.to_string()),
                        ty: substituted,
                    });
                }

                for &method_id in &hir_iface.methods {
                    if let Some(method) = self.hir.functions.get(&method_id) {
                        if method.name == member_name {
                            let param_types: Vec<_> = method
                                .params
                                .iter()
                                .map(|p| substitute_type_params(&p.ty, &iface_tp_ids, type_args))
                                .collect();
                            let ret_type =
                                substitute_type_params(&method.return_type, &iface_tp_ids, type_args);
                            return Ok(TypedExpr {
                                kind: ExprKind::Member(Box::new(typed_obj), member_name.to_string()),
                                ty: ResolvedType::Function(param_types, Box::new(ret_type)),
                            });
                        }
                    }
                }

                Err(vec![ValidationError {
                    kind: ErrorKind::UndefinedMember {
                        ty: hir_iface.name.clone(),
                        member: member_name.to_string(),
                    },
                    module: module_name.to_string(),
                    context: Some(context.to_string()),
                }])
            }

            other => Err(vec![ValidationError {
                kind: ErrorKind::UndefinedMember {
                    ty: self.format_type(other),
                    member: member_name.to_string(),
                },
                module: module_name.to_string(),
                context: Some(context.to_string()),
            }]),
        }
    }

    /// Simple capture analysis: collect variables from the enclosing scope referenced in the body
    fn collect_captures(
        &self,
        body: &ast::Block,
        enclosing_scope: &LocalScope,
    ) -> Vec<hir::HirCapture> {
        let mut captures = Vec::new();
        let mut seen = std::collections::HashSet::new();
        self.scan_captures_block(body, enclosing_scope, &mut captures, &mut seen);
        captures
    }

    fn scan_captures_block(
        &self,
        block: &ast::Block,
        scope: &LocalScope,
        captures: &mut Vec<hir::HirCapture>,
        seen: &mut std::collections::HashSet<String>,
    ) {
        for stmt in &block.statements {
            self.scan_captures_stmt(stmt, scope, captures, seen);
        }
        if let Some(ret) = &block.returns {
            self.scan_captures_expr(ret, scope, captures, seen);
        }
    }

    fn scan_captures_stmt(
        &self,
        stmt: &ast::Statement,
        scope: &LocalScope,
        captures: &mut Vec<hir::HirCapture>,
        seen: &mut std::collections::HashSet<String>,
    ) {
        match stmt {
            ast::Statement::VarDecl(_, _, Some(e)) => self.scan_captures_expr(e, scope, captures, seen),
            ast::Statement::Return(Some(e)) => self.scan_captures_expr(e, scope, captures, seen),
            ast::Statement::Expr(e) => self.scan_captures_expr(e, scope, captures, seen),
            ast::Statement::While(cond, body) => {
                self.scan_captures_expr(cond, scope, captures, seen);
                self.scan_captures_block(body, scope, captures, seen);
            }
            ast::Statement::For(_, iter, body) => {
                self.scan_captures_expr(iter, scope, captures, seen);
                self.scan_captures_block(body, scope, captures, seen);
            }
            _ => {}
        }
    }

    fn scan_captures_expr(
        &self,
        expr: &ast::Expr,
        scope: &LocalScope,
        captures: &mut Vec<hir::HirCapture>,
        seen: &mut std::collections::HashSet<String>,
    ) {
        match expr {
            ast::Expr::Variable(name) => {
                if !seen.contains(name) {
                    if let Some(ty) = scope.lookup(name) {
                        seen.insert(name.clone());
                        captures.push(hir::HirCapture {
                            name: name.clone(),
                            ty: ty.clone(),
                        });
                    }
                }
            }
            ast::Expr::BinaryOp(l, _, r) => {
                self.scan_captures_expr(l, scope, captures, seen);
                self.scan_captures_expr(r, scope, captures, seen);
            }
            ast::Expr::UnaryOp(_, e) => self.scan_captures_expr(e, scope, captures, seen),
            ast::Expr::Call(callee, _, args) => {
                self.scan_captures_expr(callee, scope, captures, seen);
                for a in args {
                    self.scan_captures_expr(a, scope, captures, seen);
                }
            }
            ast::Expr::Member(obj, _) => self.scan_captures_expr(obj, scope, captures, seen),
            ast::Expr::If(cond, then_b, else_b) => {
                self.scan_captures_expr(cond, scope, captures, seen);
                self.scan_captures_block(then_b, scope, captures, seen);
                if let Some(eb) = else_b {
                    self.scan_captures_block(eb, scope, captures, seen);
                }
            }
            ast::Expr::As(e, _) | ast::Expr::Is(e, _) => {
                self.scan_captures_expr(e, scope, captures, seen);
            }
            ast::Expr::LiteralList(elems) => {
                for e in elems {
                    self.scan_captures_expr(e, scope, captures, seen);
                }
            }
            ast::Expr::LiteralMap(entries) => {
                for (k, v) in entries {
                    self.scan_captures_expr(k, scope, captures, seen);
                    self.scan_captures_expr(v, scope, captures, seen);
                }
            }
            ast::Expr::StructInit(_, fields) => {
                for (_, v) in fields {
                    self.scan_captures_expr(v, scope, captures, seen);
                }
            }
            ast::Expr::Block(b) => self.scan_captures_block(b, scope, captures, seen),
            ast::Expr::FunctionLiteral(_, _, _, body) => {
                self.scan_captures_block(body, scope, captures, seen);
            }
            ast::Expr::Literal(_) => {}
        }
    }
}

fn is_numeric(ty: &ResolvedType) -> bool {
    matches!(
        ty,
        ResolvedType::Primitive(
            hir::PrimitiveType::Int8
                | hir::PrimitiveType::Int16
                | hir::PrimitiveType::Int32
                | hir::PrimitiveType::Int64
                | hir::PrimitiveType::Uint8
                | hir::PrimitiveType::Uint16
                | hir::PrimitiveType::Uint32
                | hir::PrimitiveType::Uint64
                | hir::PrimitiveType::Float32
                | hir::PrimitiveType::Float64
        )
    )
}

fn convert_binary_op(op: &ast::BinaryOperator) -> hir::BinaryOperator {
    match op {
        ast::BinaryOperator::Add => hir::BinaryOperator::Add,
        ast::BinaryOperator::Sub => hir::BinaryOperator::Sub,
        ast::BinaryOperator::Mul => hir::BinaryOperator::Mul,
        ast::BinaryOperator::Div => hir::BinaryOperator::Div,
        ast::BinaryOperator::Mod => hir::BinaryOperator::Mod,
        ast::BinaryOperator::And => hir::BinaryOperator::And,
        ast::BinaryOperator::Or => hir::BinaryOperator::Or,
        ast::BinaryOperator::Eq => hir::BinaryOperator::Eq,
        ast::BinaryOperator::Neq => hir::BinaryOperator::Neq,
        ast::BinaryOperator::Lt => hir::BinaryOperator::Lt,
        ast::BinaryOperator::Le => hir::BinaryOperator::Le,
        ast::BinaryOperator::Gt => hir::BinaryOperator::Gt,
        ast::BinaryOperator::Ge => hir::BinaryOperator::Ge,
    }
}

fn convert_unary_op(op: &ast::UnaryOperator) -> hir::UnaryOperator {
    match op {
        ast::UnaryOperator::Neg => hir::UnaryOperator::Neg,
        ast::UnaryOperator::Not => hir::UnaryOperator::Not,
    }
}
