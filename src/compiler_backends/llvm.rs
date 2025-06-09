use anyhow::{Result, Context};
use std::collections::{HashMap, HashSet};
use super::{Env, Expr, Stmt, Symbol, CompileTarget};
use super::lift_global_decls;

fn wrap_symbol_name(name: &Symbol) -> String {
    format!("_{}", name)
}

#[derive(Default)]
pub struct LLVMCompiler {
    current_function_code: Vec<String>,
    global_declarations: Vec<String>,
    loop_stack: Vec<(String, String)>,
    reg_counter: usize,
    label_counter: usize,
    local_vars: Vec<HashMap<String, String>>,
    known_functions: HashSet<String>,
    known_globals: HashMap<String, String>,
}

impl LLVMCompiler {
    fn fresh_reg(&mut self) -> String {
        let r = format!("%r{}", self.reg_counter);
        self.reg_counter += 1;
        r
    }

    fn fresh_label(&mut self, prefix: &str) -> String {
        let l = format!("{}.{}", prefix, self.label_counter);
        self.label_counter += 1;
        l
    }

    fn emit(&mut self, line: &str) {
        self.current_function_code.push(line.to_string());
    }

    fn push_scope(&mut self) {
        self.local_vars.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.local_vars.pop();
    }

    fn add_local_var(&mut self, name: &str, alloca_reg: &str) {
        if let Some(scope) = self.local_vars.last_mut() {
            scope.insert(name.to_string(), alloca_reg.to_string());
        }
    }

    fn lookup_var(&self, name: &str) -> Option<String> {
        // Check local scopes first
        for scope in self.local_vars.iter().rev() {
            if let Some(a) = scope.get(name) {
                return Some(a.clone());
            }
        }

        // Then check global variables
        if let Some(g) = self.known_globals.get(name) {
            return Some(format!("@{}", g));
        }

        None
    }

    fn load_var(&mut self, name: &str) -> Result<String> {
        if let Some(ptr) = self.lookup_var(name) {
            let reg = self.fresh_reg();
            self.emit(&format!("  {reg} = load i64, i64* {ptr}"));
            Ok(reg)
        } else {
            Err(anyhow::anyhow!("Loaded undefined variable: {}", name))
        }
    }

    fn store_var(&mut self, name: &str, value_reg: &str) -> Result<()> {
        if let Some(ptr) = self.lookup_var(name) {
            self.emit(&format!("  store i64 {value_reg}, i64* {ptr}"));
            Ok(())
        } else {
            Err(anyhow::anyhow!("Stored to undefined variable: {}", name))
        }
    }

    fn begin_function(&mut self, name: &str, args: &[Symbol]) {
        self.current_function_code.clear();
        self.local_vars.clear();
        self.push_scope();

        let arg_list = args.iter().enumerate()
            .map(|(i, _)| format!("i64 %arg{}", i))
            .collect::<Vec<_>>()
            .join(", ");

        self.emit(&format!("define i64 @{name}({arg_list}) {{"));
        // Create allocas for arguments
        for (i, a) in args.iter().enumerate() {
            let alloc_reg = self.fresh_reg();
            let arg_name = wrap_symbol_name(a);
            self.emit(&format!("  {alloc_reg} = alloca i64"));
            self.add_local_var(&arg_name, &alloc_reg);
            self.emit(&format!("  store i64 %arg{i}, i64* {alloc_reg}"));
        }
    }

    fn end_function(&mut self) -> String {
        self.emit("}");
        self.current_function_code.join("\n")
    }

    fn bool_to_int(&self, b: bool) -> i64 {
        if b { 1 } else { 0 }
    }

    fn is_known_function(&self, name: &str) -> bool {
        self.known_functions.contains(name)
    }

    fn create_headers(&self, procs: &Vec<Stmt>, output: &mut dyn std::fmt::Write) {
        // Add known functions
        for proc in procs {
            if let Stmt::DeclareProc { name, args, .. } = proc.strip_annotations() {
                let fn_name = wrap_symbol_name(name);
                let arg_types = args.iter().map(|_| "i64").collect::<Vec<_>>().join(", ");
                writeln!(output, "declare i64 @{fn_name}({arg_types})").unwrap();
            }
        }
    }
}

impl CompileTarget for LLVMCompiler {
    fn has_extern(&self, _name: &str) -> bool {
        true
    }

    fn compile_main(&mut self, program: Stmt) -> Result<String> {
        let (procs, stmts) = lift_global_decls(vec![program]);

        // Add known functions for extern and declared procedures
        for p in &procs {
            match p.strip_annotations() {
                Stmt::DeclareProc { name, .. } => {
                    let fn_name = wrap_symbol_name(name);
                    self.known_functions.insert(fn_name);
                }
                Stmt::ExternProc { name, .. } => {
                    let fn_name = wrap_symbol_name(name);
                    self.known_functions.insert(fn_name);
                }
                Stmt::DeclareVar { name, is_static: true, value } => {
                    let global_name = wrap_symbol_name(name);
                    self.known_globals.insert(global_name.clone(), global_name.clone());
                    let initial_val = match *value.clone() {
                        Expr::Int(v) => v.to_string(),
                        Expr::Char(c) => (c as i64).to_string(),
                        Expr::Float(f) => (f64::to_bits(f) as i64).to_string(),
                        Expr::Bool(b) => self.bool_to_int(b).to_string(),
                        // _ => "0".to_string() // fallback for non-constant initializers
                        other => return Err(anyhow::anyhow!("Unsupported global initializer type: {other:?}").context("Global variables must be initialized with a constant expression, e.g. an integer, character, float, or boolean")),
                    };
                    self.global_declarations.push(format!("@{} = global i64 {}", global_name, initial_val));
                }
                _ => {}
            }
        }

        let mut code = String::new();
        code.push_str("; ModuleID = 'mage_output'\n");
        #[cfg(target_os = "macos")]
        code.push_str("target triple = \"arm64-apple-macosx14.0.0\"\n\n");
        #[cfg(target_os = "linux")]
        code.push_str("target triple = \"x86_64-unknown-linux-gnu\"\n\n");
        #[cfg(target_os = "windows")]
        code.push_str("target triple = \"x86_64-pc-windows-msvc\"\n\n");

        // self.create_headers(&procs, &mut code);

        // External function declarations
        for p in &procs {
            if let Stmt::ExternProc { name, args, .. } = p.strip_annotations() {
                let fn_name = wrap_symbol_name(name);
                let arg_types = args.iter().map(|_| "i64").collect::<Vec<_>>().join(", ");
                code.push_str(&format!("declare i64 @{fn_name}({arg_types})\n"));
            }
        }

        // Emit global variables
        for gdecl in &self.global_declarations {
            code.push_str(&format!("{}\n", gdecl));
        }

        code.push('\n');

        // Define user-defined procedures
        for p in &procs {
            if let Stmt::DeclareProc { name, args, body } = p.strip_annotations() {
                let fn_name = wrap_symbol_name(name);
                self.begin_function(&fn_name, args);
                self.compile_stmt(body, &Env::default())?;
                // If no explicit return, return 0
                self.emit("  ret i64 0");
                let fn_def = self.end_function();
                code.push_str(&fn_def);
                code.push_str("\n\n");
            }
        }

        // Main function
        self.begin_function("main", &[]);
        self.compile_stmt(&Stmt::Block(stmts), &Env::default())?;
        self.emit("  ret i64 0");
        let main_def = self.end_function();
        code.push_str(&main_def);
        code.push('\n');

        Ok(code)
    }

    fn compile_lib(&mut self, program: Stmt) -> Result<String> {
        let (procs, stmts) = lift_global_decls(vec![program]);

        // Add known functions for extern and declared procedures
        for p in &procs {
            match p.strip_annotations() {
                Stmt::DeclareProc { name, .. } => {
                    let fn_name = wrap_symbol_name(name);
                    self.known_functions.insert(fn_name);
                }
                Stmt::ExternProc { name, .. } => {
                    let fn_name = wrap_symbol_name(name);
                    self.known_functions.insert(fn_name);
                }
                Stmt::DeclareVar { name, is_static: true, value } => {
                    let global_name = wrap_symbol_name(name);
                    self.known_globals.insert(global_name.clone(), global_name.clone());
                    let initial_val = match *value.clone() {
                        Expr::Int(v) => v.to_string(),
                        Expr::Char(c) => (c as i64).to_string(),
                        Expr::Float(f) => (f64::to_bits(f) as i64).to_string(),
                        Expr::Bool(b) => self.bool_to_int(b).to_string(),
                        // _ => "0".to_string() // fallback for non-constant initializers
                        other => return Err(anyhow::anyhow!("Unsupported global initializer type: {other:?}").context("Global variables must be initialized with a constant expression, e.g. an integer, character, float, or boolean")),
                    };
                    self.global_declarations.push(format!("@{} = global i64 {}", global_name, initial_val));
                }
                _ => {}
            }
        }
        let mut code = String::new();
        code.push_str("; ModuleID = 'mage_output'\n");
        #[cfg(target_os = "macos")]
        code.push_str("target triple = \"arm64-apple-macosx14.0.0\"\n\n");
        #[cfg(target_os = "linux")]
        code.push_str("target triple = \"x86_64-unknown-linux-gnu\"\n\n");
        #[cfg(target_os = "windows")]
        code.push_str("target triple = \"x86_64-pc-windows-msvc\"\n\n");

        // self.create_headers(&procs, &mut code);

        // External function declarations
        for p in &procs {
            if let Stmt::ExternProc { name, args, .. } = p.strip_annotations() {
                let fn_name = wrap_symbol_name(name);
                let arg_types = args.iter().map(|_| "i64").collect::<Vec<_>>().join(", ");
                code.push_str(&format!("declare i64 @{fn_name}({arg_types})\n"));
            }
        }

        // Emit global variables
        for gdecl in &self.global_declarations {
            code.push_str(&format!("{}\n", gdecl));
        }

        code.push('\n');

        // Define user-defined procedures
        for p in &procs {
            if let Stmt::DeclareProc { name, args, body } = p.strip_annotations() {
                let fn_name = wrap_symbol_name(name);
                self.begin_function(&fn_name, args);
                self.compile_stmt(body, &Env::default())?;
                // If no explicit return, return 0
                self.emit("  ret i64 0");
                let fn_def = self.end_function();
                code.push_str(&fn_def);
                code.push_str("\n\n");
            }
        }

        Ok(code)
    }

    fn compile_expr(&mut self, expr: &Expr, _env: &Env) -> Result<String> {
        match expr {
            Expr::Annotated(metadata, inner) => self.compile_expr(inner, _env)
                .context(format!("Failed to compile annotated expression: {}", metadata)),
            Expr::If(cond, then, else_) => {
                let cond_val = self.compile_expr(cond, _env)?;
                let then_label = self.fresh_label("then");
                let else_label = self.fresh_label("else");
                let end_label = self.fresh_label("endif");
                let result_reg = self.fresh_reg();

                let result_alloca = self.fresh_reg();
                self.emit(&format!("  {result_alloca} = alloca i64"));

                let cond_cmp = self.fresh_reg();
                self.emit(&format!("  {cond_cmp} = icmp ne i64 {cond_val}, 0"));
                self.emit(&format!("  br i1 {cond_cmp}, label %{then_label}, label %{else_label}"));

                self.emit(&format!("{then_label}:"));
                let then_val = self.compile_expr(then, _env)?;
                self.emit(&format!("  store i64 {then_val}, i64* {result_alloca}"));
                self.emit(&format!("  br label %{end_label}"));

                self.emit(&format!("{else_label}:"));
                let else_val = self.compile_expr(else_, _env)?;
                self.emit(&format!("  store i64 {else_val}, i64* {result_alloca}"));
                self.emit(&format!("  br label %{end_label}"));

                self.emit(&format!("{end_label}:"));
                self.emit(&format!("  {result_reg} = load i64, i64* {result_alloca}"));
                Ok(result_reg)
            }
            Expr::Int(value) => Ok(value.to_string()),
            Expr::Char(value) => Ok((*value as i64).to_string()),
            Expr::Float(value) => Ok((f64::to_bits(*value) as i64).to_string()),
            Expr::Bool(value) => Ok(self.bool_to_int(*value).to_string()),
            Expr::Var(name) => {
                let var_name = wrap_symbol_name(name);
                if self.is_known_function(&var_name) {
                    // This is a function symbol
                    Ok(format!("@{}", var_name))
                } else {
                    // This is a variable
                    self.load_var(&var_name)
                }
            }
            Expr::Ref(name) => {
                if let Some(alloca) = self.lookup_var(&wrap_symbol_name(name)) {
                    let ptr_int = self.fresh_reg();
                    self.emit(&format!("  {ptr_int} = ptrtoint i64* {alloca} to i64"));
                    Ok(ptr_int)
                } else {
                    Err(anyhow::anyhow!("Referenced pointer to undefined variable: {}", name))
                }
            }
            Expr::App(func, args) => {
                let func_val = self.compile_expr(func, _env)?;
                let arg_vals = args
                    .iter()
                    .map(|a| self.compile_expr(a, _env))
                    .collect::<Result<Vec<_>>>()?;
                let result_reg = self.fresh_reg();
                let mut arg_strings = Vec::new();

                for arg_val in &arg_vals {
                    let arg_str = if arg_val.starts_with('@') {
                        let casted = self.fresh_reg();
                        self.emit(&format!(
                            "  {casted} = ptrtoint i64 ({})* {} to i64",
                            "i64", // ajusta si sabes el tipo real
                            arg_val
                        ));
                        casted
                    } else {
                        arg_val.clone()
                    };
                    arg_strings.push(format!("i64 {}", arg_str));
                }

                let arg_list = arg_strings.join(", ");

                if func_val.starts_with('@') {
                    self.emit(&format!("  {result_reg} = call i64 {func_val}({arg_list})"));
                } else {
                    let casted_fn = self.fresh_reg();
                    let fn_type = format!("i64({})", vec!["i64"; arg_vals.len()].join(", "));
                    self.emit(&format!("  {casted_fn} = inttoptr i64 {func_val} to {fn_type}*"));
                    self.emit(&format!("  {result_reg} = call i64 {casted_fn}({arg_list})"));
                }
                Ok(result_reg)
            },
            Expr::Array(values) => {
                let num_elems = values.len();
                
                // First, compile each array element to a register:
                let compiled_values = values
                    .iter()
                    .map(|val| self.compile_expr(val, _env))
                    .collect::<Result<Vec<_>>>()?;
            
                // 1) Allocate space on the stack: [N x i64]
                let arr_alloca = self.fresh_reg();
                self.emit(&format!("  {arr_alloca} = alloca [{} x i64]", num_elems));
            
                // 2) Store each element at the correct index
                for (i, val_reg) in compiled_values.iter().enumerate() {
                    let element_ptr = self.fresh_reg();
            
                    // Get pointer to arr_alloca[0][i]
                    self.emit(&format!(
                        "  {element_ptr} = getelementptr inbounds [{} x i64], [{} x i64]* {arr_alloca}, i64 0, i64 {}",
                        num_elems,
                        num_elems,
                        i
                    ));
            
                    self.emit(&format!("  store i64 {val_reg}, i64* {element_ptr}"));
                }
            
                // 3) Get pointer to the first element 
                let arr_ptr = self.fresh_reg();
                self.emit(&format!(
                    "  {arr_ptr} = getelementptr inbounds [{} x i64], [{} x i64]* {arr_alloca}, i64 0, i64 0",
                    num_elems,
                    num_elems
                ));
            
                // 4) Turn that pointer into an i64 (so our IR sees it as a regular integer)
                let arr_ptr_int = self.fresh_reg();
                self.emit(&format!("  {arr_ptr_int} = ptrtoint i64* {arr_ptr} to i64"));
            
                // Return that pointer-as-integer
                Ok(arr_ptr_int)
            }
        }
    }

    fn compile_stmt(&mut self, stmt: &Stmt, _env: &Env) -> Result<String> {
        match stmt {
            Stmt::Annotated(metadata, stmt) => {
                let stmt = self.compile_stmt(stmt, _env)
                    .context(format!("Failed to compile annotated statement: {}", metadata))?;
                Ok(format!("/* {} */\n{}", metadata, stmt))
            }
            Stmt::Expr(expr) => {
                let _ = self.compile_expr(expr, _env)?;
                Ok("".to_string())
            }
            Stmt::Return(value) => {
                let val = self.compile_expr(value, _env)?;
                self.emit(&format!("  ret i64 {val}"));
                Ok("".to_string())
            }
            Stmt::Continue => {
                if let Some((cont_label, _)) = self.loop_stack.last() {
                    self.emit(&format!("  br label %{cont_label}"));
                    Ok("".to_string())
                } else {
                    Err(anyhow::anyhow!("'continue' outside of loop"))
                }
            }
            Stmt::Break => {
                if let Some((_, break_label)) = self.loop_stack.last() {
                    self.emit(&format!("  br label %{break_label}"));
                    Ok("".to_string())
                } else {
                    Err(anyhow::anyhow!("'break' outside of loop"))
                }
            }
            Stmt::DeclareVar { name, is_static, value } => {
                if *is_static {
                    let global_name = wrap_symbol_name(name);
                    // Attempt to get a compile-time constant initial value
                    // For simplicity, handle known constant types:
                    let initial_val = match *value.clone() {
                        Expr::Int(v) => v.to_string(),
                        Expr::Char(c) => (c as i64).to_string(),
                        Expr::Float(f) => (f64::to_bits(f) as i64).to_string(),
                        Expr::Bool(b) => self.bool_to_int(b).to_string(),
                        other => return Err(anyhow::anyhow!("Unsupported global initializer type: {other:?}").context("Global variables must be initialized with a constant expression, e.g. an integer, character, float, or boolean")),
                    };

                    self.known_globals.insert(global_name.clone(), global_name.clone());
                    self.global_declarations.push(format!("@{} = global i64 {}", global_name, initial_val));
                    Ok("".to_string())
                } else {
                    let val = self.compile_expr(value, _env)?;
                    let var_name = wrap_symbol_name(name);
                    let alloca_reg = self.fresh_reg();
                    self.emit(&format!("  {alloca_reg} = alloca i64"));
                    self.add_local_var(&var_name, &alloca_reg);
                    self.emit(&format!("  store i64 {val}, i64* {alloca_reg}"));
                    Ok("".to_string())
                }
            }
            Stmt::DeclareProc { .. } => {
                // Already handled in compile()
                Ok("".to_string())
            }
            Stmt::ExternProc { .. } => {
                // Already handled in compile()
                Ok("".to_string())
            }
            Stmt::AssignVar(name, value) => {
                let val = self.compile_expr(value, _env)?;
                let var_name = wrap_symbol_name(name);
                self.store_var(&var_name, &val)?;
                Ok("".to_string())
            }
            Stmt::AssignRef(dst, src) => {
                let dst_val = self.compile_expr(dst, _env)?;
                let src_val = self.compile_expr(src, _env)?;
                let ptr_reg = self.fresh_reg();
                self.emit(&format!("  {ptr_reg} = inttoptr i64 {dst_val} to i64*"));
                self.emit(&format!("  store i64 {src_val}, i64* {ptr_reg}"));
                Ok("".to_string())
            }
            Stmt::While(cond, body) => {
                let cond_label = self.fresh_label("while.cond");
                let body_label = self.fresh_label("while.body");
                let end_label  = self.fresh_label("while.end");

                self.loop_stack.push((cond_label.clone(), end_label.clone()));

                self.emit(&format!("  br label %{cond_label}"));
                self.emit(&format!("{cond_label}:"));
                let cond_val = self.compile_expr(cond, _env)?;
                let cond_cmp = self.fresh_reg();
                self.emit(&format!("  {cond_cmp} = icmp ne i64 {cond_val}, 0"));
                self.emit(&format!("  br i1 {cond_cmp}, label %{body_label}, label %{end_label}"));

                self.emit(&format!("{body_label}:"));
                self.compile_stmt(body, _env)?;
                self.emit(&format!("  br label %{cond_label}"));

                self.emit(&format!("{end_label}:"));

                self.loop_stack.pop();
                Ok("".to_string())
            }
            Stmt::If(cond, then, else_) => {
                let then_label = self.fresh_label("if.then");
                let else_label = self.fresh_label("if.else");
                let end_label  = self.fresh_label("if.end");

                let cond_val = self.compile_expr(cond, _env)?;
                let cond_cmp = self.fresh_reg();
                self.emit(&format!("  {cond_cmp} = icmp ne i64 {cond_val}, 0"));
                self.emit(&format!("  br i1 {cond_cmp}, label %{then_label}, label %{else_label}"));

                self.emit(&format!("{then_label}:"));
                self.compile_stmt(then, _env)?;
                self.emit(&format!("  br label %{end_label}"));

                self.emit(&format!("{else_label}:"));
                self.compile_stmt(else_, _env)?;
                self.emit(&format!("  br label %{end_label}"));

                self.emit(&format!("{end_label}:"));
                Ok("".to_string())
            }
            Stmt::Block(stmts) => {
                self.push_scope();
                for s in stmts {
                    self.compile_stmt(s, _env)?;
                }
                self.pop_scope();
                Ok("".to_string())
            }
        }
    }
}