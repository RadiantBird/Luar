//! 型検査でインスタンスメソッドの呼び出しと分かった `obj.method(args)` を、
//! メソッド呼び出し(`obj:method(args)`)へ書き換える。
//!
//! Luarは `.` と `:` を区別せず、`self` は自動で渡す。どのメソッドが `self` を取るかは
//! 受け手の型で決まるので、型を知るチェッカーが呼び出しの位置(ノードのアドレス)を記録し、
//! ここで一括して書き換える。アドレスは比べるだけで、参照はしない。

use crate::ast::{Expr, InterpolatedPart, Member, Stmt, TableField};
use std::collections::HashSet;

/// `sites` にあるノードの `Call` を `MethodCall` にする。
pub fn rewrite(stmts: &mut [Stmt], sites: &HashSet<usize>) {
    let mut rewriter = Rewriter { sites };
    rewriter.stmts(stmts);
}

struct Rewriter<'a> {
    sites: &'a HashSet<usize>,
}

impl Rewriter<'_> {
    fn stmts(&mut self, stmts: &mut [Stmt]) {
        for stmt in stmts {
            self.stmt(stmt);
        }
    }

    fn stmt(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::Local { values, .. } | Stmt::Const { values, .. } | Stmt::Return(values) => {
                self.exprs(values)
            }
            Stmt::Assign { targets, values } => {
                self.exprs(targets);
                self.exprs(values);
            }
            Stmt::FunctionDecl { body, .. } | Stmt::Do { body } => self.stmts(body),
            Stmt::While { cond, body } | Stmt::Repeat { body, cond } => {
                self.expr(cond);
                self.stmts(body);
            }
            Stmt::If { clauses, else_body } => {
                for clause in clauses {
                    self.expr(&mut clause.cond);
                    self.stmts(&mut clause.body);
                }
                if let Some(body) = else_body {
                    self.stmts(body);
                }
            }
            Stmt::NumericFor {
                start,
                limit,
                step,
                body,
                ..
            } => {
                self.expr(start);
                self.expr(limit);
                if let Some(step) = step {
                    self.expr(step);
                }
                self.stmts(body);
            }
            Stmt::GenericFor { iters, body, .. } => {
                self.exprs(iters);
                self.stmts(body);
            }
            Stmt::ExprStmt(expr) => self.expr(expr),
            Stmt::ClassDecl(class) => {
                let members = class
                    .top_level_members
                    .iter_mut()
                    .chain(class.blocks.iter_mut().flat_map(|block| &mut block.members));
                for member in members {
                    match member {
                        Member::Field(field) => {
                            if let Some(value) = &mut field.value {
                                self.expr(value);
                            }
                        }
                        Member::Method(method) => {
                            if let Some(body) = &mut method.body {
                                self.stmts(body);
                            }
                        }
                    }
                }
            }
            Stmt::Break
            | Stmt::Continue
            | Stmt::Goto { .. }
            | Stmt::Label { .. }
            | Stmt::RawLua54(_)
            | Stmt::ImportDecl { .. }
            | Stmt::DeclareStmt { .. }
            | Stmt::TypeAlias { .. }
            | Stmt::DeclareFunction { .. } => {}
        }
    }

    fn exprs(&mut self, exprs: &mut [Expr]) {
        for expr in exprs {
            self.expr(expr);
        }
    }

    fn expr(&mut self, expr: &mut Expr) {
        // 書き換えても、このノード自身の位置は変わらない。
        let address = expr as *const Expr as usize;
        if self.sites.contains(&address) {
            self.rewrite_call(expr);
        }
        match expr {
            Expr::Call { callee, args } => {
                self.expr(callee);
                self.exprs(args);
            }
            Expr::MethodCall { obj, args, .. } => {
                self.expr(obj);
                self.exprs(args);
            }
            Expr::Field { obj, .. }
            | Expr::Unop { expr: obj, .. }
            | Expr::Cast { expr: obj, .. } => self.expr(obj),
            Expr::Bind { value, .. } => self.expr(value),
            Expr::Index { obj, key } => {
                self.expr(obj);
                self.expr(key);
            }
            Expr::Binop { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Table(fields) => {
                for field in fields {
                    match field {
                        TableField::Index { key, value } => {
                            self.expr(key);
                            self.expr(value);
                        }
                        TableField::Name { value, .. } | TableField::Value(value) => {
                            self.expr(value)
                        }
                    }
                }
            }
            Expr::Function { body, .. } => self.stmts(body),
            Expr::If(if_expr) => {
                for clause in &mut if_expr.clauses {
                    self.expr(&mut clause.cond);
                    self.stmts(&mut clause.branch.statements);
                    self.expr(&mut clause.branch.result);
                }
                self.stmts(&mut if_expr.else_branch.statements);
                self.expr(&mut if_expr.else_branch.result);
            }
            Expr::InterpolatedString(parts) => {
                for part in parts {
                    if let InterpolatedPart::Expr(inner) = part {
                        self.expr(inner);
                    }
                }
            }
            Expr::Nil
            | Expr::True
            | Expr::False
            | Expr::Number(_)
            | Expr::Str(_)
            | Expr::Vararg
            | Expr::Ident { .. }
            | Expr::SelfExpr
            | Expr::SuperExpr => {}
        }
    }

    /// `obj.name(args)` → `obj:name(args)`。形が違えばそのまま。
    fn rewrite_call(&mut self, expr: &mut Expr) {
        let Expr::Call { callee, .. } = expr else {
            return;
        };
        if !matches!(callee.as_ref(), Expr::Field { .. }) {
            return;
        }
        let Expr::Call { callee, args } = std::mem::replace(expr, Expr::Nil) else {
            return;
        };
        match *callee {
            Expr::Field { obj, name } => {
                *expr = Expr::MethodCall {
                    obj,
                    method: name,
                    args,
                };
            }
            other => {
                *expr = Expr::Call {
                    callee: Box::new(other),
                    args,
                };
            }
        }
    }
}
