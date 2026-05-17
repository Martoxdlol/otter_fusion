use crate::{
    hir::{ExprKind, HirBlock, HirStatement, TypedExpr},
    lower::{Lower, builder::FnBuilder, subst::Subst},
    mir::{AssignValue, Operand, Stmt, Terminator},
};

impl Lower {
    pub fn lower_stmt(&mut self, s: &HirStatement, subst: &Subst, b: &mut FnBuilder) {
        match s {
            HirStatement::VarDecl(name, ty, init) => {
                let mty = self.lower_type(ty, subst);
                let local = b.new_local(Some(name.clone()), mty);
                if let Some(e) = init {
                    let op = self.lower_expr(e, subst, b);
                    b.push_stmt(Stmt::Assign(local, AssignValue::Use(op)));
                }
                b.bind(name.clone(), local);
            }

            HirStatement::Return(opt_e) => {
                let op = opt_e.as_ref().map(|e| self.lower_expr(e, subst, b));
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
                todo!()
                // self.lower_for(name, iter_e, body, subst, b);
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
        if matches!(
            b.blocks[&b.current_block].terminator,
            Terminator::Unreachable
        ) {
            b.terminate(Terminator::Goto(head_bb));
        }

        // Continue lowering after the loop.
        b.switch_to(exit_bb);
    }
}
