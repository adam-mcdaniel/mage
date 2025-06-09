use std::{
    collections::{BTreeMap, HashMap}, sync::{Arc, RwLock}
};
use super::{Expr, Stmt};
use anyhow::{Context, Result};
use super::Symbol;

pub trait Interface {
    fn has_extern(&self, name: &str) -> bool;
    fn external_call(&mut self, name: &str, args: Vec<i64>) -> Result<i64>;
}

pub struct Interpreter<T: Interface> {
    static_vars: Arc<RwLock<BTreeMap<Symbol, i64>>>,
    env: BTreeMap<Symbol, i64>,
    procs: BTreeMap<Symbol, (Vec<Symbol>, Box<Stmt>)>,
    extern_procs: HashMap<Symbol, (Vec<Symbol>, Option<Box<Stmt>>)>,
    return_value: i64,
    returned: bool,
    loop_broken: bool,
    loop_continued: bool,
    interface: T,
}

impl<T: Interface> Interpreter<T> {
    pub fn new(interface: T) -> Self {
        Self {
            static_vars: Arc::new(RwLock::new(BTreeMap::new())),
            env: BTreeMap::new(),
            procs: BTreeMap::new(),
            extern_procs: HashMap::new(),
            interface,
            returned: false,
            return_value: 0,
            loop_broken: false,
            loop_continued: false,
        }
    }

    pub fn partial_run(&mut self, program: &Stmt) -> Result<()> {
        self.eval_stmt(program)?;
        Ok(())
    }

    pub fn run(mut self, program: &Stmt) -> Result<T> {
        self.eval_stmt(program)?;
        Ok(self.interface)
    }

    pub fn declare_static_var(&mut self, name: Symbol, value: i64) {
        // Check if the static variable already exists
        if self.static_vars.read().unwrap().contains_key(&name) {
            return;
        }

        // Add the static variable to the static variables
        self.static_vars.write().unwrap().insert(name, value);
    }

    pub fn write_static_var(&mut self, name: Symbol, value: i64) {
        // Check if the static variable exists
        if !self.static_vars.read().unwrap().contains_key(&name) {
            return;
        }

        // Write the value to the static variable
        self.static_vars.write().unwrap().insert(name, value);
    }

    pub fn read_static_var(&mut self, name: Symbol) -> i64 {
        // Check if the static variable exists
        if !self.static_vars.read().unwrap().contains_key(&name) {
            return 0;
        }

        // Read the value from the static variable
        *self.static_vars.read().unwrap().get(&name).unwrap()
    }

    unsafe fn reference_variable(&mut self, var: &Symbol) -> *mut i64 {
        // First, check if the variable is a static variable
        if self.static_vars.read().unwrap().contains_key(var) {
            // Return a pointer to the mapped value in the static variables
            return self.static_vars.read().unwrap().get(var).unwrap() as *const i64 as *mut i64
        }

        // Check if the variable is in the environment
        if self.env.contains_key(var) {
            // Return a pointer to the mapped value in the environment
            return self.env.get(var).unwrap() as *const i64 as *mut i64
        }

        // Return a null pointer
        std::ptr::null_mut()
    }

    pub fn is_static_var(&self, name: &Symbol) -> bool {
        self.static_vars.read().unwrap().contains_key(name)
    }

    pub fn add_proc(&mut self, name: Symbol, args: Vec<Symbol>, body: Box<Stmt>) {
        self.procs.insert(name, (args, body));
    }

    pub fn add_external_proc(&mut self, name: Symbol, args: Vec<Symbol>, body: Option<Box<Stmt>>) {
        self.extern_procs.insert(name, (args, body));
    }

    fn eval_expr(&mut self, expr: &Expr) -> Result<i64> {
        // Annotated(Metadata, Box<Self>),
        // If(Box<Self>, Box<Self>, Box<Self>),
        // Int(i64),
        // Char(char),
        // Float(f64),
        // Bool(bool),
        // Var(Symbol),
        // Ref(Symbol),
        // App(Box<Self>, Vec<Self>),
        Ok(match expr {
            Expr::Annotated(_, expr) => self.eval_expr(expr)?,
            Expr::Int(value) => *value,
            Expr::Char(value) => *value as i64,
            Expr::Float(value) => f64::to_bits(*value) as i64,
            Expr::Bool(value) => *value as i64,
            Expr::Var(name) => {
                if self.env.contains_key(name) {
                    self.env[name]
                } else if self.static_vars.read().unwrap().contains_key(name) {
                    *self.static_vars.read().unwrap().get(name).unwrap()
                } else if let Some((_arg_names, _)) = self.procs.get(name) {
                    self.procs.keys().position(|key| key == name).unwrap() as i64
                } else {
                    anyhow::bail!("Unknown variable: {:?}", name);
                }
            },
            Expr::Ref(name) => {
                // Get a reference to a variable
                // Get the item in the environment
                unsafe {
                    self.reference_variable(name) as i64
                }
            },
            Expr::App(proc, args) => {
                // Check if the proc is an external proc
                let proc_number = if let Expr::Var(name) = &**proc {
                    if let Some((arg_names, _)) = self.procs.get(name) {
                        if arg_names.len() != args.len() {
                            anyhow::bail!("Incorrect number of arguments");
                        }

                        self.procs.keys().position(|key| key == name).unwrap() as i64
                    } else if let Some((arg_names, body)) = self.extern_procs.get(name).cloned() {
                        if arg_names.len() != args.len() {
                            anyhow::bail!("Incorrect number of arguments");
                        }
                        let args = args.iter().map(|arg| self.eval_expr(arg)).collect::<Result<_>>()?;
                        if self.interface.has_extern(name.as_ref()) {
                            return self.interface.external_call(name.as_ref(), args);
                        } else if let Some(body) = body {
                            let mut new_env = BTreeMap::new();
                            for (name, value) in arg_names.iter().zip(args.iter()) {
                                new_env.insert(name.clone(), *value);
                            }
                            let old_env = std::mem::replace(&mut self.env, new_env);
                            self.eval_stmt(&body)?;
                            self.env = old_env;
                            return Ok(self.return_value);
                        } else {
                            anyhow::bail!("Cannot find external proc: {:?}", name);
                        }
                    } else {
                        anyhow::bail!("Unknown or undeclared proc: {:?}", name);
                    }
                } else {
                    self.eval_expr(proc)?
                };


                // Get the proc from the environment
                let (arg_names, body) = self.procs.values().nth(proc_number as usize).unwrap().clone();

                let mut new_env = BTreeMap::new();
                for (name, value) in arg_names.iter().zip(args.iter()) {
                    new_env.insert(name.clone(), self.eval_expr(value)?);
                }
                let old_env = std::mem::replace(&mut self.env, new_env);
                self.eval_stmt(&body)?;
                self.env = old_env;
                // Undo the `returned` flag
                self.returned = false;
                self.return_value
            }
            Expr::If(condition, then_expr, else_expr) => {
                if self.eval_expr(condition)? != 0 {
                    self.eval_expr(then_expr)?
                } else {
                    self.eval_expr(else_expr)?
                }
            }
            Expr::Array(values) => {
                let mut result = Vec::new();
                for value in values {
                    // Evaluate the value
                    result.push(self.eval_expr(value)?);
                }

                // Now, return a pointer to the array
                let ptr = Box::into_raw(result.into_boxed_slice()) as *mut i64;
                ptr as i64
            }
        })
    }

    fn eval_stmt(&mut self, stmt: &Stmt) -> Result<()> {
        if self.loop_broken || self.loop_continued || self.returned {
            return Ok(());
        }
        match stmt {
            Stmt::Annotated(metadata, stmt) => {
                self.eval_stmt(stmt).context(format!("At {metadata}"))?
            }
            Stmt::Expr(expr) => {
                self.return_value = self.eval_expr(expr)?;
            }
            Stmt::Return(expr) => {
                self.return_value = self.eval_expr(expr)?;
                self.returned = true;
            }
            Stmt::Continue => {
                self.loop_continued = true;
            }
            Stmt::Break => {
                self.loop_broken = true;
            }
            Stmt::DeclareVar { name, is_static, value } => {
                let result = self.eval_expr(value)?;
                if *is_static {
                    self.declare_static_var(name.clone(), result);
                } else {
                    self.env.insert(name.clone(), result);
                }
            }
            Stmt::DeclareProc { name, args, body, .. } => {
                self.add_proc(name.clone(), args.clone(), body.clone());
            }
            Stmt::ExternProc { name, args, body } => {
                self.add_external_proc(name.clone(), args.clone(), body.clone());
            }
            Stmt::AssignVar(name, value) => {
                let result = self.eval_expr(value)?;
                if self.env.contains_key(name) {
                    self.env.insert(name.clone(), result);
                } else if self.static_vars.read().unwrap().contains_key(name) {
                    self.write_static_var(name.clone(), result);
                } else {
                    anyhow::bail!("Unknown variable: {:?}", name);
                }
            }

            Stmt::AssignRef(reference, value) => {
                // Evaluate the reference to get the index of the variable
                unsafe {
                    let reference = self.eval_expr(reference)? as *const i64 as *mut i64;
                    // Assign the value to the variable
                    let result = self.eval_expr(value)?;

                    *reference = result;
                }
            }

            Stmt::While(condition, body) => {
                while self.eval_expr(condition)? != 0 {
                    self.eval_stmt(body)?;
                    if self.loop_broken || self.returned {
                        self.loop_broken = false;
                        break;
                    }
                    if self.loop_continued {
                        self.loop_continued = false;
                        continue;
                    }
                }
            }
            Stmt::If(condition, then_body, else_body) => {
                if self.eval_expr(condition)? != 0 {
                    self.eval_stmt(then_body)?;
                } else {
                    self.eval_stmt(else_body)?;
                }
            }
            Stmt::Block(stmts) => {
                for stmt in stmts {
                    self.eval_stmt(stmt)?;
                    if self.loop_broken || self.loop_continued || self.returned {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}