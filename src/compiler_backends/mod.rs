use anyhow::Result;
use super::{Env, Expr, Stmt, Symbol};

mod llvm;
pub use llvm::*;

mod c;
pub use c::*;

mod cword;
pub use cword::*;

fn wrap_symbol_name(name: &Symbol) -> String {
    format!("_{}", name)
}

pub(super) fn lift_global_decls(stmts: Vec<Stmt>) -> (Vec<Stmt>, Vec<Stmt>) {
    let mut new_stmts: Vec<Stmt> = Vec::new();
    let mut globals = Vec::new();
    for stmt in stmts {
        match stmt {
            Stmt::Annotated(metadata, stmt) => {
                let (new_procs, new_new_stmts) = lift_global_decls(vec![*stmt]);
                globals.extend(new_procs.into_iter().map(|stmt| stmt.annotate(metadata.clone())));
                new_stmts.extend(new_new_stmts);
            }
            Stmt::DeclareProc { .. } => {
                globals.push(stmt);
            }
            Stmt::ExternProc { .. } => {
                globals.push(stmt);
            }
            Stmt::DeclareVar { is_static: true, .. } => {
                globals.push(stmt);
            }
            Stmt::Block(stmts) => {
                let (new_procs, new_new_stmts) = lift_global_decls(stmts);
                globals.extend(new_procs);
                new_stmts.extend(new_new_stmts);
            }
            stmt => new_stmts.push(stmt),
        }
    }
    (globals, new_stmts)
}

pub trait CompileTarget {
    fn has_extern(&self, name: &str) -> bool;
    fn compile_main(&mut self, program: Stmt) -> Result<String>;
    fn compile_lib(&mut self, program: Stmt) -> Result<String>;
    fn compile_expr(&mut self, expr: &Expr, env: &Env) -> Result<String>;
    fn compile_stmt(&mut self, stmt: &Stmt, env: &Env) -> Result<String>;
}
