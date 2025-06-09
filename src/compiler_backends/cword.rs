use anyhow::Result;
use super::{CompileTarget, Stmt, Expr, Env, lift_global_decls, wrap_symbol_name};
use std::fmt::Write;

#[derive(Default)]
pub struct CWordCompiler;

fn to_c_func_type(arg_count: usize) -> String {
    format!("intptr_t (*)({})", (0..arg_count).map(|_| "intptr_t").collect::<Vec<_>>().join(", "))
}

impl CWordCompiler {
    fn create_headers(&self, stmt: &Stmt, output: &mut dyn std::fmt::Write) {
        println!("Creating headers for CWordCompiler");
        match stmt.strip_annotations() {
            Stmt::Block(stmts) => {
                for stmt in stmts {
                    self.create_headers(&stmt, output);
                }
            }
            Stmt::DeclareProc { name, args, .. } => {
                writeln!(output, "intptr_t {}({});", wrap_symbol_name(name), args.iter().map(|arg| format!("intptr_t {}", wrap_symbol_name(arg))).collect::<Vec<_>>().join(", ")).unwrap();
            }
            _ => {
                // For other statements, we don't need to create headers
            }
        }
    }
}

impl CompileTarget for CWordCompiler {
    fn has_extern(&self, _name: &str) -> bool {
        true
    }

    fn compile_main(&mut self, stmt: Stmt) -> Result<String> {
        let mut prelude = String::from("#include <stdint.h>\n#include <stddef.h>\n#include <string.h>\nintptr_t mage_as_int(double x) {{ return *(intptr_t*)&x; }}\n\n#if __has_include(\"ffi.h\")\n#include \"ffi.h\"\n#endif\n\n/* BEGIN PROCEDURES */");
        // Create headers for the procedures
        self.create_headers(&stmt, &mut prelude);

        let (procs, program) = lift_global_decls(vec![stmt]);
        let _env = Env::default();
        // self.compile_stmt(&Stmt::Block(program), &_env)
        

        let mut result = String::new();

        // Compile the lifted procedures
        for stmt in procs {
            writeln!(prelude, "{}", self.compile_stmt(&stmt, &_env)?)?;
        }

        // Compile the main program
        write!(result, "{}", self.compile_stmt(&Stmt::Block(program), &_env)?)?;

        Ok(format!("{}\n/* BEGIN MAIN */\nint main() {{\n{}\n}}", prelude, result))
    }

    fn compile_lib(&mut self, stmt: Stmt) -> Result<String> {
        let mut prelude = String::from("#include <stdint.h>\n#include <stddef.h>\n#include <string.h>\nintptr_t mage_as_int(double x) {{ return *(intptr_t*)&x; }}\n\n#if __has_include(\"ffi.h\")\n#include \"ffi.h\"\n#endif\n\n/* BEGIN PROCEDURES */");
        // Create headers for the procedures
        self.create_headers(&stmt, &mut prelude);
        let (procs, program) = lift_global_decls(vec![stmt]);
        let _env = Env::default();

        // Compile the lifted procedures
        for stmt in procs {
            writeln!(prelude, "{}", self.compile_stmt(&stmt, &_env)?)?;
        }

        // Compile the main program
        write!(prelude, "{}", self.compile_stmt(&Stmt::Block(program), &_env)?)?;

        Ok(prelude)
    }
    
    fn compile_expr(&mut self, expr: &Expr, _env: &Env) -> Result<String> {
        match expr {
            Expr::Annotated(_, expr) => self.compile_expr(expr, _env),
            Expr::If(cond, then, else_) => {
                let cond = self.compile_expr(cond, _env)?;
                let then = self.compile_expr(then, _env)?;
                let else_ = self.compile_expr(else_, _env)?;
                Ok(format!("{}? {} : {}", cond, then, else_))
            }
            Expr::Int(value) => Ok(format!("{value:?}")),
            Expr::Char(value) => Ok(format!("{value:?}")),
            Expr::Float(value) => Ok(format!("mage_as_int({value:?})")),
            Expr::Bool(value) => Ok(if *value { "1" } else { "0" }.to_string()),
            Expr::Var(name) => Ok(wrap_symbol_name(name)),
            Expr::Ref(name) => Ok(format!("((intptr_t)&{})", wrap_symbol_name(name))),
            Expr::App(func, args) => {
                let func = self.compile_expr(func, _env)?;
                let args = args.iter().map(|arg| Ok(format!("(intptr_t)({})", self.compile_expr(arg, _env)?))).collect::<Result<Vec<_>>>()?;
                Ok(format!("(({}){})({})", to_c_func_type(args.len()), func, args.join(", ")))
                // let args = args.iter().map(|arg| self.compile_expr(arg, _env)).collect::<Result<Vec<_>>>()?;
                // Ok(format!("{}({})", func, args.join(", ")))
            },
            Expr::Array(values) => {
                let values = values.iter().map(|value| self.compile_expr(value, _env)).collect::<Result<Vec<_>>>()?;
                // Allocate on the stack
                Ok(format!("(intptr_t)(intptr_t*)(intptr_t[]){{ {} }}", values.join(", ")))
            }
        }
    }

    fn compile_stmt(&mut self, stmt: &Stmt, _env: &Env) -> Result<String> {
        match stmt {
            Stmt::Annotated(metadata, stmt) => {
                let stmt = self.compile_stmt(stmt, _env)?;
                Ok(format!("/* {} */\n{}", metadata, stmt))
            }
            Stmt::Expr(expr) => Ok(format!("{};", self.compile_expr(expr, _env)?)),
            Stmt::Return(value) => {
                let value = self.compile_expr(value, _env)?;
                Ok(format!("return {};", value))
            }
            Stmt::Continue => Ok("continue;".to_string()),
            Stmt::Break => Ok("break;".to_string()),
            Stmt::DeclareVar { name, is_static, value } => {
                let name = wrap_symbol_name(name);
                let value = self.compile_expr(value, _env)?;
                if *is_static {
                    Ok(format!("static intptr_t {} = {};", name, value))
                } else {
                    Ok(format!("intptr_t {} = {};", name, value))
                }
            }
            Stmt::DeclareProc { name, args, body } => {
                let name = wrap_symbol_name(name);
                let new_env = _env.new_scope();
                let args = args.iter().map(|arg| format!("intptr_t {}", wrap_symbol_name(arg))).collect::<Vec<_>>().join(", ");
                let body = self.compile_stmt(body, &new_env)?;
                Ok(format!("intptr_t {}({}) {{\n{}\nreturn 0;\n}}", name, args, body))
            }
            Stmt::ExternProc { name, args, body } => {
                if self.has_extern(name.as_ref()) {
                    let name = wrap_symbol_name(name);
                    let args = args.iter().map(|arg| format!("intptr_t {}", arg)).collect::<Vec<_>>().join(", ");
                    Ok(format!("intptr_t {}({});", name, args))
                } else if let Some(body) = body {
                    // Use the body as a fallback
                    self.compile_stmt(&Stmt::DeclareProc { name: name.clone(), args: args.clone(), body: body.clone() }, _env)
                } else {
                    Err(anyhow::anyhow!("Undefined external procedure: {}", name))
                }
            }
            Stmt::AssignVar(name, value) => {
                let name = wrap_symbol_name(name);
                let value = self.compile_expr(value, _env)?;
                Ok(format!("{} = {};", name, value))
            }
            Stmt::AssignRef(dst, src) => {
                let dst = self.compile_expr(dst, _env)?;
                let src = self.compile_expr(src, _env)?;
                Ok(format!("*(intptr_t*){} = {};", dst, src))
            }
            Stmt::While(cond, body_) => {
                let cond = self.compile_expr(cond, _env)?;
                let body = self.compile_stmt(body_, _env)?;
                Ok(format!("while ({}) {{\n{}\n}}", cond, body))
            }
            Stmt::If(cond, then, else_) => {
                let cond = self.compile_expr(cond, _env)?;
                let then = self.compile_stmt(then, _env)?;
                let else_ = self.compile_stmt(else_, _env)?;
                Ok(format!("if ({}) {{\n{}\n}} else {{\n{}\n}}", cond, then, else_))
            }

            Stmt::Block(stmts) => {
                let stmts = stmts.iter().map(|stmt| self.compile_stmt(stmt, _env)).collect::<Result<Vec<_>>>()?;
                Ok(stmts.join("\n"))
            }
        }
    }
}