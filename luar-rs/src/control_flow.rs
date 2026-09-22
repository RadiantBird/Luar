use crate::ast::{Expr, Member, Param, Program, Stmt};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowError {
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionIr {
    pub name: String,
    pub blocks: Vec<BasicBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicBlock {
    pub id: usize,
    pub instructions: Vec<String>,
    pub terminator: Terminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Terminator {
    Jump(usize),
    Branch {
        condition: String,
        then_block: usize,
        else_block: usize,
    },
    Return(Vec<String>),
    Stop,
}

#[derive(Clone)]
struct JumpSite {
    name: String,
    line: usize,
    locals: HashSet<usize>,
}

#[derive(Clone)]
struct LabelSite {
    line: usize,
    locals: HashSet<usize>,
}

struct Validator {
    errors: Vec<FlowError>,
    labels: HashMap<String, LabelSite>,
    gotos: Vec<JumpSite>,
    next_local: usize,
}

impl Validator {
    fn new() -> Self {
        Self {
            errors: Vec::new(),
            labels: HashMap::new(),
            gotos: Vec::new(),
            next_local: 0,
        }
    }

    fn local(&mut self) -> usize {
        let id = self.next_local;
        self.next_local += 1;
        id
    }

    fn validate_function(&mut self, body: &[Stmt], params: &[Param]) {
        let mut locals = HashSet::new();
        for param in params {
            if matches!(param, Param::Named { .. }) {
                let id = self.local();
                locals.insert(id);
            }
        }
        self.walk_block(body, &mut locals, 0);

        for jump in std::mem::take(&mut self.gotos) {
            let Some(label) = self.labels.get(&jump.name) else {
                self.errors.push(FlowError {
                    line: jump.line,
                    message: format!("undefined label '{}'", jump.name),
                });
                continue;
            };
            if !label.locals.is_subset(&jump.locals) {
                self.errors.push(FlowError {
                    line: jump.line,
                    message: format!(
                        "goto '{}' jumps into the scope of a local declared after the goto",
                        jump.name
                    ),
                });
            }
        }
        self.labels.clear();
    }

    fn nested_function(&mut self, body: &[Stmt], params: &[Param]) {
        let mut nested = Validator::new();
        nested.validate_function(body, params);
        self.errors.extend(nested.errors);
    }

    fn walk_block(&mut self, body: &[Stmt], locals: &mut HashSet<usize>, loop_depth: usize) {
        for stmt in body {
            match stmt {
                Stmt::Local { names, values, .. } | Stmt::Const { names, values, .. } => {
                    for value in values {
                        self.walk_expr(value);
                    }
                    for _ in names {
                        let id = self.local();
                        locals.insert(id);
                    }
                }
                Stmt::FunctionDecl { params, body, .. } => self.nested_function(body, params),
                Stmt::Assign { targets, values } => {
                    for expr in targets.iter().chain(values) {
                        self.walk_expr(expr);
                    }
                }
                Stmt::Do { body } => self.walk_child(body, locals, loop_depth),
                Stmt::While { cond, body } => {
                    self.walk_expr(cond);
                    self.walk_child(body, locals, loop_depth + 1);
                }
                Stmt::Repeat { body, cond } => {
                    let mut child = locals.clone();
                    self.walk_block(body, &mut child, loop_depth + 1);
                    self.walk_expr(cond);
                }
                Stmt::If { clauses, else_body } => {
                    for clause in clauses {
                        self.walk_expr(&clause.cond);
                        self.walk_child(&clause.body, locals, loop_depth);
                    }
                    if let Some(body) = else_body {
                        self.walk_child(body, locals, loop_depth);
                    }
                }
                Stmt::NumericFor {
                    start,
                    limit,
                    step,
                    body,
                    ..
                } => {
                    self.walk_expr(start);
                    self.walk_expr(limit);
                    if let Some(step) = step {
                        self.walk_expr(step);
                    }
                    let mut child = locals.clone();
                    let id = self.local();
                    child.insert(id);
                    self.walk_block(body, &mut child, loop_depth + 1);
                }
                Stmt::GenericFor { names, iters, body } => {
                    for expr in iters {
                        self.walk_expr(expr);
                    }
                    let mut child = locals.clone();
                    for _ in names {
                        let id = self.local();
                        child.insert(id);
                    }
                    self.walk_block(body, &mut child, loop_depth + 1);
                }
                Stmt::Return(values) => {
                    for value in values {
                        self.walk_expr(value);
                    }
                }
                Stmt::Break if loop_depth == 0 => self.errors.push(FlowError {
                    line: 1,
                    message: "break is only allowed inside a loop".to_string(),
                }),
                Stmt::Continue if loop_depth == 0 => self.errors.push(FlowError {
                    line: 1,
                    message: "continue is only allowed inside a loop".to_string(),
                }),
                Stmt::Goto { label, line } => self.gotos.push(JumpSite {
                    name: label.clone(),
                    line: *line,
                    locals: locals.clone(),
                }),
                Stmt::Label { name, line } => {
                    if let Some(previous) = self.labels.get(name) {
                        self.errors.push(FlowError {
                            line: *line,
                            message: format!(
                                "duplicate label '{}' (first declared on line {})",
                                name, previous.line
                            ),
                        });
                    } else {
                        self.labels.insert(
                            name.clone(),
                            LabelSite {
                                line: *line,
                                locals: locals.clone(),
                            },
                        );
                    }
                }
                Stmt::ExprStmt(expr) => self.walk_expr(expr),
                Stmt::ClassDecl(class) => {
                    for member in class
                        .top_level_members
                        .iter()
                        .chain(class.blocks.iter().flat_map(|block| &block.members))
                    {
                        match member {
                            Member::Field(field) => {
                                if let Some(value) = &field.value {
                                    self.walk_expr(value);
                                }
                            }
                            Member::Method(method) => {
                                if let Some(body) = &method.body {
                                    self.nested_function(body, &method.params);
                                }
                            }
                        }
                    }
                }
                Stmt::Break
                | Stmt::Continue
                | Stmt::ImportDecl { .. }
                | Stmt::DeclareStmt { .. }
                | Stmt::RawLua54(_) => {}
            }
        }
    }

    fn walk_child(&mut self, body: &[Stmt], locals: &HashSet<usize>, loop_depth: usize) {
        let mut child = locals.clone();
        self.walk_block(body, &mut child, loop_depth);
    }

    fn walk_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Function { params, body, .. } => self.nested_function(body, params),
            Expr::InterpolatedString(parts) => {
                for part in parts {
                    if let crate::ast::InterpolatedPart::Expr(expr) = part {
                        self.walk_expr(expr);
                    }
                }
            }
            Expr::Field { obj, .. } => self.walk_expr(obj),
            Expr::Index { obj, key } => {
                self.walk_expr(obj);
                self.walk_expr(key);
            }
            Expr::Call { callee, args } => {
                self.walk_expr(callee);
                for arg in args {
                    self.walk_expr(arg);
                }
            }
            Expr::MethodCall { obj, args, .. } => {
                self.walk_expr(obj);
                for arg in args {
                    self.walk_expr(arg);
                }
            }
            Expr::Unop { expr, .. } => self.walk_expr(expr),
            Expr::Binop { left, right, .. } => {
                self.walk_expr(left);
                self.walk_expr(right);
            }
            Expr::Table(fields) => {
                for field in fields {
                    match field {
                        crate::ast::TableField::Index { key, value } => {
                            self.walk_expr(key);
                            self.walk_expr(value);
                        }
                        crate::ast::TableField::Name { value, .. }
                        | crate::ast::TableField::Value(value) => self.walk_expr(value),
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
}

pub fn validate(program: &Program) -> Vec<FlowError> {
    let mut validator = Validator::new();
    validator.validate_function(&program.stmts, &[]);
    let mut errors = validator.errors;
    validate_label_layout(&program.stmts, false, &mut errors);
    errors
}

/// Labels inside nested lexical blocks are deliberately rejected for now.  Lua
/// permits some of those jumps, but treating every function as one global label
/// namespace would also accidentally allow illegal sibling-block jumps.  A
/// nested `goto` to a top-level label remains valid (and is how loop exits are
/// represented on the Luau dispatcher path).
fn validate_label_layout(body: &[Stmt], nested: bool, errors: &mut Vec<FlowError>) {
    for stmt in body {
        match stmt {
            Stmt::Label { line, name } if nested => errors.push(FlowError {
                line: *line,
                message: format!(
                    "label '{}' is nested in a lexical block; nested labels are not supported yet",
                    name
                ),
            }),
            Stmt::FunctionDecl { body, .. } => validate_label_layout(body, false, errors),
            Stmt::Do { body }
            | Stmt::While { body, .. }
            | Stmt::Repeat { body, .. }
            | Stmt::NumericFor { body, .. }
            | Stmt::GenericFor { body, .. } => validate_label_layout(body, true, errors),
            Stmt::If { clauses, else_body } => {
                for clause in clauses {
                    validate_label_layout(&clause.body, true, errors);
                }
                if let Some(body) = else_body {
                    validate_label_layout(body, true, errors);
                }
            }
            Stmt::ClassDecl(class) => {
                for member in class
                    .top_level_members
                    .iter()
                    .chain(class.blocks.iter().flat_map(|block| &block.members))
                {
                    if let Member::Method(method) = member {
                        if let Some(body) = &method.body {
                            validate_label_layout(body, false, errors);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn lower(program: &Program) -> Vec<FunctionIr> {
    let mut functions = vec![lower_function("<chunk>", &program.stmts)];
    collect_functions(&program.stmts, &mut functions);
    functions
}

fn collect_functions(body: &[Stmt], output: &mut Vec<FunctionIr>) {
    for stmt in body {
        match stmt {
            Stmt::FunctionDecl { name, body, .. } => {
                output.push(lower_function(name, body));
                collect_functions(body, output);
            }
            Stmt::Do { body }
            | Stmt::While { body, .. }
            | Stmt::Repeat { body, .. }
            | Stmt::NumericFor { body, .. }
            | Stmt::GenericFor { body, .. } => collect_functions(body, output),
            Stmt::If { clauses, else_body } => {
                for clause in clauses {
                    collect_functions(&clause.body, output);
                }
                if let Some(body) = else_body {
                    collect_functions(body, output);
                }
            }
            _ => {}
        }
    }
}

fn lower_function(name: &str, body: &[Stmt]) -> FunctionIr {
    let mut labels = HashMap::new();
    for (index, stmt) in body.iter().enumerate() {
        if let Stmt::Label { name, .. } = stmt {
            labels.insert(name.clone(), index);
        }
    }
    let blocks = body
        .iter()
        .enumerate()
        .map(|(index, stmt)| {
            let next = index + 1;
            let terminator = match stmt {
                Stmt::Goto { label, .. } => labels
                    .get(label)
                    .copied()
                    .map(Terminator::Jump)
                    .unwrap_or(Terminator::Stop),
                Stmt::Return(values) => {
                    Terminator::Return(values.iter().map(|value| format!("{value:?}")).collect())
                }
                Stmt::If { clauses, else_body } => Terminator::Branch {
                    condition: clauses
                        .first()
                        .map(|clause| format!("{:?}", clause.cond))
                        .unwrap_or_else(|| "false".to_string()),
                    then_block: next,
                    else_block: if else_body.is_some() { next + 1 } else { next },
                },
                _ if next < body.len() => Terminator::Jump(next),
                _ => Terminator::Stop,
            };
            BasicBlock {
                id: index,
                instructions: vec![stmt_name(stmt).to_string()],
                terminator,
            }
        })
        .collect();
    FunctionIr {
        name: name.to_string(),
        blocks,
    }
}

fn stmt_name(stmt: &Stmt) -> &'static str {
    match stmt {
        Stmt::Local { .. } => "local",
        Stmt::Const { .. } => "const",
        Stmt::FunctionDecl { .. } => "function",
        Stmt::Assign { .. } => "assign",
        Stmt::Do { .. } => "do",
        Stmt::While { .. } => "while",
        Stmt::Repeat { .. } => "repeat",
        Stmt::If { .. } => "if",
        Stmt::NumericFor { .. } => "numeric-for",
        Stmt::GenericFor { .. } => "generic-for",
        Stmt::Return(_) => "return",
        Stmt::Break => "break",
        Stmt::Continue => "continue",
        Stmt::Goto { .. } => "goto",
        Stmt::Label { .. } => "label",
        Stmt::ExprStmt(_) => "expression",
        Stmt::ClassDecl(_) => "class",
        Stmt::ImportDecl { .. } => "import-type",
        Stmt::DeclareStmt { .. } => "declare",
        Stmt::RawLua54(_) => "raw-lua54",
    }
}

pub fn dump(program: &Program) -> String {
    let mut output = String::new();
    for function in lower(program) {
        output.push_str(&format!("function {}\n", function.name));
        for block in function.blocks {
            output.push_str(&format!(
                "  block {}: {:?} -> {:?}\n",
                block.id, block.instructions, block.terminator
            ));
        }
    }
    output
}
