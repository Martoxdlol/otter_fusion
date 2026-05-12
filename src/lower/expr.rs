use crate::{
    hir::{ExprKind, TypedExpr},
    lower::{Lower, builder::FnBuilder, subst::Subst},
    mir::Operand,
};

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
            ExprKind::Is(_, _) => unimplemented!(),
            ExprKind::If(c, t, e) => self.lower_if(c, t, e.as_deref(), &expr.ty, subst, b),
            ExprKind::Block(blk) => self.lower_block_expr(blk, &expr.ty, subst, b),
            ExprKind::LiteralList(items) => self.lower_list_lit(items, &expr.ty, subst, b),
            ExprKind::LiteralMap(pairs) => self.lower_map_lit(pairs, &expr.ty, subst, b),
            ExprKind::FunctionLiteral(..) => unimplemented!("Phase 3: closures"),
        }
    }
}
