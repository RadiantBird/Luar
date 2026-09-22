use crate::Target;
use crate::ast::*;
use std::collections::{HashMap, HashSet};

const OPERATOR_META: &[(&str, &str)] = &[
    ("==", "__eq"),
    ("<", "__lt"),
    ("<=", "__le"),
    ("+", "__add"),
    ("-", "__sub"),
    ("*", "__mul"),
    ("/", "__div"),
    ("//", "__idiv"),
    ("%", "__mod"),
    ("^", "__pow"),
    ("..", "__concat"),
    ("#", "__len"),
];

#[derive(Clone)]
enum MethodKind {
    Instance,
    Static,
    Operator,
}

#[derive(Clone)]
struct MethodRecord {
    kind: MethodKind,
    access: Access,
    owner: String,
}

struct ClassRegistry {
    methods: HashMap<String, MethodRecord>,
    parent: Option<String>,
    new_params: Option<Vec<Param>>,
    new_access: Option<Access>,
}

pub struct Codegen {
    out: Vec<String>,
    indent: usize,
    type_env: Vec<HashMap<String, String>>,
    current_class: Option<String>,
    registry: HashMap<String, ClassRegistry>,
    target: Target,
    next_generated_name: usize,
    continue_wrappers: Vec<Option<String>>,
    dispatcher: Option<(String, String)>,
    reserved_names: HashSet<String>,
}

struct LoweredExpr {
    prelude: Vec<String>,
    expr: String,
}

impl Codegen {
    pub fn new() -> Self {
        Self::for_target(Target::Luau)
    }

    pub fn for_target(target: Target) -> Self {
        Codegen {
            out: Vec::new(),
            indent: 0,
            type_env: vec![HashMap::new()],
            current_class: None,
            registry: HashMap::new(),
            target,
            next_generated_name: 0,
            continue_wrappers: Vec::new(),
            dispatcher: None,
            reserved_names: HashSet::new(),
        }
    }

    pub fn generate(&mut self, program: &Program) -> String {
        self.out.clear();
        self.indent = 0;
        self.type_env = vec![HashMap::new()];
        self.registry = self.build_registry(program);
        self.reserved_names = collect_program_names(program);

        self.emit_function_body(&program.stmts);
        self.out.join("\n")
    }

    // ─── Class registry ───────────────────────────────────────────────────────

    fn build_registry(&self, program: &Program) -> HashMap<String, ClassRegistry> {
        let mut reg = HashMap::new();
        for stmt in &program.stmts {
            let Stmt::ClassDecl(decl) = stmt else {
                continue;
            };
            let mut methods = HashMap::new();
            let mut new_params = None;
            let mut new_access = None;

            for (access, member) in Self::flatten_members(decl) {
                if let Member::Method(m) = member {
                    let kind = if m.is_operator {
                        MethodKind::Operator
                    } else if m.is_static {
                        MethodKind::Static
                    } else {
                        MethodKind::Instance
                    };
                    if m.name == "new" && m.is_static {
                        new_params = Some(m.params.clone());
                        new_access = Some(access.clone());
                    }
                    methods.insert(
                        m.name.clone(),
                        MethodRecord {
                            kind,
                            access,
                            owner: decl.name.clone(),
                        },
                    );
                }
            }
            reg.insert(
                decl.name.clone(),
                ClassRegistry {
                    methods,
                    parent: decl.parent.clone(),
                    new_params,
                    new_access,
                },
            );
        }
        reg
    }

    fn flatten_members(decl: &ClassDecl) -> Vec<(Access, Member)> {
        let mut result = Vec::new();
        for m in &decl.top_level_members {
            result.push((Access::Private, m.clone()));
        }
        for block in &decl.blocks {
            for m in &block.members {
                result.push((block.access.clone(), m.clone()));
            }
        }
        result
    }

    fn parent_name(&self) -> Option<String> {
        self.current_class
            .as_ref()
            .and_then(|c| self.registry.get(c))
            .and_then(|r| r.parent.clone())
    }

    fn lookup_method(&self, class_name: &str, method_name: &str) -> Option<MethodRecord> {
        let mut current = Some(class_name.to_string());
        while let Some(name) = current {
            if let Some(reg) = self.registry.get(&name) {
                if let Some(k) = reg.methods.get(method_name) {
                    return Some(k.clone());
                }
                current = reg.parent.clone();
            } else {
                break;
            }
        }
        None
    }

    fn private_method_call(
        &self,
        class_name: &str,
        method_name: &str,
        receiver: &str,
        args: &str,
    ) -> Option<String> {
        let method = self.lookup_method(class_name, method_name)?;
        if method.access != Access::Private
            || self.current_class.as_deref() != Some(method.owner.as_str())
        {
            return None;
        }
        let all_args = match method.kind {
            MethodKind::Instance | MethodKind::Operator => {
                if args.is_empty() {
                    receiver.to_string()
                } else {
                    format!("{receiver}, {args}")
                }
            }
            MethodKind::Static => args.to_string(),
        };
        Some(format!("{method_name}({all_args})"))
    }

    // ─── Type environment ─────────────────────────────────────────────────────

    fn push_scope(&mut self) {
        self.type_env.push(HashMap::new());
    }
    fn pop_scope(&mut self) {
        self.type_env.pop();
    }
    fn set_type(&mut self, name: &str, class: &str) {
        if let Some(top) = self.type_env.last_mut() {
            top.insert(name.to_string(), class.to_string());
        }
    }
    fn resolve_type(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident { name: n, .. } => {
                for frame in self.type_env.iter().rev() {
                    if let Some(t) = frame.get(n) {
                        return Some(t.clone());
                    }
                }
                None
            }
            Expr::SelfExpr => self.current_class.clone(),
            Expr::Call { callee, .. } => {
                if let Expr::Field { obj, name } = callee.as_ref() {
                    if name == "new" {
                        if let Expr::Ident { name: cn, .. } = obj.as_ref() {
                            if self.registry.contains_key(cn) {
                                return Some(cn.clone());
                            }
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    // ─── Output helpers ───────────────────────────────────────────────────────

    fn line(&mut self, s: &str) {
        if s.is_empty() {
            self.out.push(String::new());
        } else {
            self.out.push(format!("{}{s}", "    ".repeat(self.indent)));
        }
    }
    fn blank(&mut self) {
        self.out.push(String::new());
    }

    fn indented(&mut self, f: impl FnOnce(&mut Self)) {
        self.indent += 1;
        f(self);
        self.indent -= 1;
    }

    fn generated_name(&mut self, suffix: &str) -> String {
        loop {
            let id = self.next_generated_name;
            self.next_generated_name += 1;
            let name = format!("__luar_{suffix}_{id}");
            if self.reserved_names.insert(name.clone()) {
                return name;
            }
        }
    }

    fn append_raw(&mut self, lines: Vec<String>) {
        self.out.extend(lines);
    }

    fn current_line(&self, text: &str) -> String {
        if text.is_empty() {
            String::new()
        } else {
            format!("{}{text}", "    ".repeat(self.indent))
        }
    }

    fn escape_string(value: &str) -> String {
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
    }

    fn emit_function_body(&mut self, body: &[Stmt]) {
        if self.target != Target::Luau || !contains_goto(body) {
            for stmt in body {
                self.emit_stmt(stmt);
            }
            return;
        }

        let state = self.generated_name("pc");
        let mut segments: Vec<(String, Vec<&Stmt>)> = vec![("__entry".to_string(), Vec::new())];
        for stmt in body {
            if let Stmt::Label { name, .. } = stmt {
                segments.push((name.clone(), Vec::new()));
            } else {
                segments.last_mut().expect("entry segment").1.push(stmt);
            }
        }

        self.line(&format!("local {state} = \"__entry\""));
        self.line(&format!("while {state} ~= nil do"));
        self.indented(|this| {
            for (index, (name, statements)) in segments.iter().enumerate() {
                let keyword = if index == 0 { "if" } else { "elseif" };
                this.line(&format!(
                    "{keyword} {state} == \"{}\" then",
                    Self::escape_string(name)
                ));
                this.indented(|this| {
                    this.line("repeat");
                    this.indented(|this| {
                        let previous = this.dispatcher.replace((state.clone(), name.clone()));
                        for stmt in statements {
                            this.emit_stmt(stmt);
                            this.emit_dispatch_guard();
                        }
                        this.dispatcher = previous;
                    });
                    this.line("until true");
                    let next = segments.get(index + 1).map(|segment| segment.0.as_str());
                    if let Some(next) = next {
                        this.line(&format!(
                            "if {state} == \"{}\" then {state} = \"{}\" end",
                            Self::escape_string(name),
                            Self::escape_string(next)
                        ));
                    } else {
                        this.line(&format!(
                            "if {state} == \"{}\" then {state} = nil end",
                            Self::escape_string(name)
                        ));
                    }
                });
            }
            this.line("else");
            this.indented(|this| {
                this.line(&format!(
                    "error(\"invalid Luar control state: \" .. tostring({state}))"
                ))
            });
            this.line("end");
        });
        self.line("end");
    }

    fn emit_dispatch_guard(&mut self) {
        if let Some((state, current)) = &self.dispatcher {
            let state = state.clone();
            let current = current.clone();
            self.line(&format!(
                "if {state} ~= \"{}\" then break end",
                Self::escape_string(&current)
            ));
        }
    }

    fn emit_loop_body(&mut self, body: &[Stmt]) {
        if self.target == Target::Lua54 && contains_continue(body) {
            let break_flag = self.generated_name("break");
            self.line(&format!("local {break_flag} = false"));
            self.line("repeat");
            self.indented(|this| {
                this.continue_wrappers.push(Some(break_flag.clone()));
                for stmt in body {
                    this.emit_stmt(stmt);
                    this.emit_dispatch_guard();
                }
                this.continue_wrappers.pop();
            });
            self.line("until true");
            self.line(&format!("if {break_flag} then break end"));
        } else {
            self.continue_wrappers.push(None);
            for stmt in body {
                self.emit_stmt(stmt);
                self.emit_dispatch_guard();
            }
            self.continue_wrappers.pop();
        }
    }

    // ─── Statements ───────────────────────────────────────────────────────────

    fn emit_expr_list(&mut self, expressions: &[Expr]) -> Vec<String> {
        let references = expressions.iter().collect::<Vec<_>>();
        let (prelude, values) = self.lower_expr_sequence(&references);
        self.append_raw(prelude);
        values
    }

    fn emit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::ClassDecl(d) => self.emit_class_decl(d),
            Stmt::Local { names, values, .. } => self.emit_local(names, values),
            Stmt::Const { names, values, .. } => {
                let ns = names.join(", ");
                let vs = self.emit_expr_list(values).join(", ");
                if self.target == Target::Lua54 {
                    let attributed = names
                        .iter()
                        .map(|name| format!("{name} <const>"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.line(&format!("local {attributed} = {vs}"));
                } else {
                    self.line(&format!("const {ns} = {vs}"));
                }
            }
            Stmt::FunctionDecl {
                name,
                params,
                body,
                is_const,
                ..
            } => {
                let prefix = if *is_const && self.target == Target::Luau {
                    "const "
                } else {
                    ""
                };
                let ps = Self::emit_params_vec(params);
                self.line(&format!("{prefix}function {name}({ps})"));
                self.indented(|s| {
                    s.push_scope();
                    s.emit_function_body(body);
                    s.pop_scope();
                });
                self.line("end");
            }
            Stmt::Assign { targets, values } => {
                let ts = self.emit_expr_list(targets).join(", ");
                let vs = self.emit_expr_list(values).join(", ");
                self.line(&format!("{ts} = {vs}"));
            }
            Stmt::Do { body } => {
                self.line("do");
                self.indented(|s| {
                    for st in body {
                        s.emit_stmt(st);
                        s.emit_dispatch_guard();
                    }
                });
                self.line("end");
            }
            Stmt::While { cond, body } => {
                if let Expr::Bind { name, value, .. } = cond {
                    self.line("while true do");
                    self.indented(|s| {
                        let value = s.emit_expr(value);
                        s.line(&format!("local {name} = {value}"));
                        s.line(&format!("if not {name} then break end"));
                        s.emit_loop_body(body);
                    });
                    self.line("end");
                } else if contains_if_expr(cond) {
                    self.line("while true do");
                    self.indented(|s| {
                        let lowered = s.lower_expr(cond);
                        s.append_raw(lowered.prelude);
                        s.line(&format!("if not ({}) then break end", lowered.expr));
                        s.emit_loop_body(body);
                    });
                    self.line("end");
                } else {
                    let c = self.emit_expr(cond);
                    self.line(&format!("while {c} do"));
                    self.indented(|s| s.emit_loop_body(body));
                    self.line("end");
                }
            }
            Stmt::Repeat { body, cond } => {
                self.line("repeat");
                self.indented(|s| s.emit_loop_body(body));
                if contains_if_expr(cond) {
                    let lowered = self.lower_expr(cond);
                    self.append_raw(lowered.prelude);
                    self.line(&format!("if {} then break end", lowered.expr));
                    self.line("until true");
                } else {
                    let c = self.emit_expr(cond);
                    self.line(&format!("until {c}"));
                }
            }
            Stmt::If { clauses, else_body } => {
                if clauses.iter().any(|clause| {
                    contains_if_expr(&clause.cond) || matches!(&clause.cond, Expr::Bind { .. })
                }) {
                    self.emit_if_statement_chain(clauses, else_body.as_deref(), 0);
                } else {
                    for (i, clause) in clauses.iter().enumerate() {
                        let kw = if i == 0 { "if" } else { "elseif" };
                        let c = self.emit_expr(&clause.cond);
                        self.line(&format!("{kw} {c} then"));
                        self.indented(|s| {
                            for st in &clause.body {
                                s.emit_stmt(st);
                                s.emit_dispatch_guard();
                            }
                        });
                    }
                    if let Some(eb) = else_body {
                        self.line("else");
                        self.indented(|s| {
                            for st in eb {
                                s.emit_stmt(st);
                                s.emit_dispatch_guard();
                            }
                        });
                    }
                    self.line("end");
                }
            }
            Stmt::NumericFor {
                name,
                start,
                limit,
                step,
                body,
            } => {
                let s = self.emit_expr(start);
                let l = self.emit_expr(limit);
                let st = step
                    .as_ref()
                    .map(|e| format!(", {}", self.emit_expr(e)))
                    .unwrap_or_default();
                self.line(&format!("for {name} = {s}, {l}{st} do"));
                self.indented(|s| s.emit_loop_body(body));
                self.line("end");
            }
            Stmt::GenericFor { names, iters, body } => {
                let ns = names.join(", ");
                let is = self.emit_expr_list(iters).join(", ");
                self.line(&format!("for {ns} in {is} do"));
                self.indented(|s| s.emit_loop_body(body));
                self.line("end");
            }
            Stmt::Return(vals) => {
                if vals.is_empty() {
                    self.line("return");
                } else {
                    let vs = self.emit_expr_list(vals).join(", ");
                    self.line(&format!("return {vs}"));
                }
            }
            Stmt::Break => {
                if let Some(Some(flag)) = self.continue_wrappers.last() {
                    let flag = flag.clone();
                    self.line(&format!("{flag} = true"));
                }
                self.line("break");
            }
            Stmt::Continue => {
                if self.target == Target::Luau {
                    self.line("continue");
                } else if self.continue_wrappers.last().is_some_and(Option::is_some) {
                    self.line("break");
                } else {
                    self.line("-- invalid continue (rejected during validation)");
                }
            }
            Stmt::Goto { label, .. } => {
                if self.target == Target::Lua54 {
                    self.line(&format!("goto {label}"));
                } else if let Some((state, _)) = &self.dispatcher {
                    let state = state.clone();
                    self.line(&format!("{state} = \"{}\"", Self::escape_string(label)));
                    self.line("break");
                }
            }
            Stmt::Label { name, .. } => {
                if self.target == Target::Lua54 {
                    self.line(&format!("::{name}::"));
                }
            }
            Stmt::RawLua54(source) => {
                for line in source.lines() {
                    self.line(line);
                }
            }
            Stmt::ExprStmt(e) => {
                let s = self.emit_expr(e);
                self.line(&s);
            }
            Stmt::ImportDecl { .. } | Stmt::DeclareStmt { .. } => {} // handled by preamble or no output
        }
    }

    fn emit_if_statement_chain(
        &mut self,
        clauses: &[IfClause],
        else_body: Option<&[Stmt]>,
        index: usize,
    ) {
        let clause = &clauses[index];
        if let Expr::Bind { name, value, .. } = &clause.cond {
            self.line("do");
            self.indented(|this| {
                let value = this.emit_expr(value);
                this.line(&format!("local {name} = {value}"));
                this.line(&format!("if {name} then"));
                this.indented(|this| {
                    for statement in &clause.body {
                        this.emit_stmt(statement);
                        this.emit_dispatch_guard();
                    }
                });
                if index + 1 < clauses.len() {
                    this.line("else");
                    this.indented(|this| {
                        this.emit_if_statement_chain(clauses, else_body, index + 1)
                    });
                } else if let Some(body) = else_body {
                    this.line("else");
                    this.indented(|this| {
                        for statement in body {
                            this.emit_stmt(statement);
                            this.emit_dispatch_guard();
                        }
                    });
                }
                this.line("end");
            });
            self.line("end");
            return;
        }
        let condition = self.lower_expr(&clause.cond);
        self.append_raw(condition.prelude);
        self.line(&format!(
            "if {} then",
            parenthesize_if_expr(&clause.cond, &condition.expr)
        ));
        self.indented(|this| {
            for statement in &clause.body {
                this.emit_stmt(statement);
                this.emit_dispatch_guard();
            }
        });
        if index + 1 < clauses.len() {
            self.line("else");
            self.indented(|this| this.emit_if_statement_chain(clauses, else_body, index + 1));
        } else if let Some(body) = else_body {
            self.line("else");
            self.indented(|this| {
                for statement in body {
                    this.emit_stmt(statement);
                    this.emit_dispatch_guard();
                }
            });
        }
        self.line("end");
    }

    fn emit_local(&mut self, names: &[String], values: &[Expr]) {
        if names.len() == 1
            && values.len() == 1
            && let Expr::Function { params, body, .. } = &values[0]
        {
            let parameters = Self::emit_params_vec(params);
            self.line(&format!("local function {}({parameters})", names[0]));
            self.indented(|this| {
                this.push_scope();
                this.emit_function_body(body);
                this.pop_scope();
            });
            self.line("end");
            return;
        }
        let ns = names.join(", ");
        if values.is_empty() {
            self.line(&format!("local {ns}"));
        } else {
            let vs = self.emit_expr_list(values).join(", ");
            self.line(&format!("local {ns} = {vs}"));
            if names.len() == 1 && values.len() == 1 {
                if let Some(t) = self.resolve_type(&values[0]) {
                    self.set_type(&names[0], &t);
                }
            }
        }
    }

    // ─── Class declaration ────────────────────────────────────────────────────

    fn emit_class_decl(&mut self, decl: &ClassDecl) {
        let name = &decl.name;
        let prev = self.current_class.clone();
        self.current_class = Some(name.clone());

        let all = Self::flatten_members(decl);
        let fields: Vec<_> = all
            .iter()
            .filter_map(|(a, m)| {
                if let Member::Field(f) = m {
                    Some((a.clone(), f.clone()))
                } else {
                    None
                }
            })
            .collect();
        let methods: Vec<_> = all
            .iter()
            .filter_map(|(a, m)| {
                if let Member::Method(m) = m {
                    Some((a.clone(), m.clone()))
                } else {
                    None
                }
            })
            .collect();

        let private_methods: Vec<_> = methods
            .iter()
            .filter(|(a, _)| a == &Access::Private)
            .collect();
        let public_methods: Vec<_> = methods
            .iter()
            .filter(|(a, _)| a == &Access::Public)
            .collect();
        let operator_methods: Vec<_> = public_methods
            .iter()
            .filter(|(_, m)| m.is_operator)
            .collect();
        let normal_methods: Vec<_> = public_methods
            .iter()
            .filter(|(_, m)| !m.is_operator)
            .collect();
        let ctor = normal_methods.iter().find(|(_, m)| m.name == "new");
        let dtor = normal_methods.iter().find(|(_, m)| m.name == "free");
        let others: Vec<_> = normal_methods
            .iter()
            .filter(|(_, m)| m.name != "new" && m.name != "free")
            .collect();

        // class table
        if let Some(parent) = &decl.parent {
            self.line(&format!(
                "local {name} = setmetatable({{}}, {{ __index = {parent} }})"
            ));
        } else {
            self.line(&format!("local {name} = {{}}"));
        }
        self.line(&format!("{name}.__index = {name}"));

        // operator metamethods
        for (_, m) in operator_methods {
            let meta = OPERATOR_META
                .iter()
                .find(|(op, _)| op == &m.operator_op)
                .map(|(_, k)| *k)
                .unwrap_or("__unknown");
            let self_param = Param::Named {
                name: "self".into(),
                ty: None,
            };
            let all_params: Vec<_> = std::iter::once(&self_param)
                .chain(m.params.iter())
                .collect();
            let ps = Self::emit_params_ref(&all_params);
            self.blank();
            self.line(&format!("{name}.{meta} = function({ps})"));
            self.indented(|s| {
                if let Some(body) = &m.body {
                    s.emit_function_body(body);
                }
            });
            self.line("end");
        }

        // private methods as local functions
        for (_, m) in &private_methods {
            if m.is_abstract {
                continue;
            }
            self.blank();
            if m.name == "new" {
                let ps = Self::emit_params_vec(&m.params);
                self.line(&format!("local function new({ps})"));
                self.indented(|s| {
                    s.line(&format!("local self = setmetatable({{}}, {name})"));
                    for (_, f) in &fields {
                        let val = f
                            .value
                            .as_ref()
                            .map(|v| s.emit_expr(v))
                            .unwrap_or_else(|| "nil".to_string());
                        s.line(&format!("self.{} = {val}", f.name));
                    }
                    if let Some(body) = &m.body {
                        s.push_scope();
                        s.emit_function_body(body);
                        s.pop_scope();
                    }
                    s.line("return self");
                });
            } else {
                let ps = if m.is_static {
                    Self::emit_params_vec(&m.params)
                } else {
                    let self_param = Param::Named {
                        name: "self".into(),
                        ty: None,
                    };
                    let all_params: Vec<_> = std::iter::once(&self_param)
                        .chain(m.params.iter())
                        .collect();
                    Self::emit_params_ref(&all_params)
                };
                self.line(&format!("local function {}({ps})", m.name));
                self.indented(|s| {
                    if let Some(body) = &m.body {
                        s.push_scope();
                        s.emit_function_body(body);
                        s.pop_scope();
                    }
                });
            }
            self.line("end");
        }

        // constructor
        if ctor.is_none() {
            if let Some(parent) = &decl.parent {
                self.emit_inherited_new(name, parent, &fields);
            }
        }
        if let Some((_, m)) = ctor {
            let ps = Self::emit_params_vec(&m.params);
            self.blank();
            self.line(&format!("function {name}.new({ps})"));
            self.indented(|s| {
                s.line(&format!("local self = setmetatable({{}}, {name})"));
                for (_, f) in &fields {
                    let val = f
                        .value
                        .as_ref()
                        .map(|v| s.emit_expr(v))
                        .unwrap_or_else(|| "nil".to_string());
                    s.line(&format!("self.{} = {val}", f.name));
                }
                if let Some(body) = &m.body {
                    s.push_scope();
                    s.emit_function_body(body);
                    s.pop_scope();
                }
                s.line("return self");
            });
            self.line("end");
        }

        // static methods
        for (_, m) in others.iter().filter(|(_, m)| m.is_static) {
            let ps = Self::emit_params_vec(&m.params);
            self.blank();
            self.line(&format!("function {name}.{}({ps})", m.name));
            self.indented(|s| {
                if let Some(body) = &m.body {
                    s.push_scope();
                    s.emit_function_body(body);
                    s.pop_scope();
                }
            });
            self.line("end");
        }

        // instance methods
        for (_, m) in others
            .iter()
            .filter(|(_, m)| !m.is_static && !m.is_abstract)
        {
            let self_param = Param::Named {
                name: "self".into(),
                ty: None,
            };
            let all_params: Vec<_> = std::iter::once(&self_param)
                .chain(m.params.iter())
                .collect();
            let ps = Self::emit_params_ref(&all_params);
            self.blank();
            self.line(&format!("function {name}.{}({ps})", m.name));
            self.indented(|s| {
                if let Some(body) = &m.body {
                    s.push_scope();
                    s.emit_function_body(body);
                    s.pop_scope();
                }
            });
            self.line("end");
        }

        // destructor
        if let Some((_, m)) = dtor {
            let self_param = Param::Named {
                name: "self".into(),
                ty: None,
            };
            let all_params: Vec<_> = std::iter::once(&self_param)
                .chain(m.params.iter())
                .collect();
            let ps = Self::emit_params_ref(&all_params);
            self.blank();
            self.line(&format!("function {name}.free({ps})"));
            self.indented(|s| {
                if let Some(body) = &m.body {
                    s.push_scope();
                    s.emit_function_body(body);
                    s.pop_scope();
                }
            });
            self.line("end");
        }

        self.current_class = prev;
    }

    fn emit_inherited_new(&mut self, name: &str, parent: &str, fields: &[(Access, FieldMember)]) {
        // find ancestor new params
        let mut ancestor_params: Option<Vec<Param>> = None;
        let mut cur = Some(parent.to_string());
        while let Some(c) = cur {
            if let Some(reg) = self.registry.get(&c) {
                if let Some(p) = &reg.new_params {
                    if reg.new_access == Some(Access::Public) {
                        ancestor_params = Some(p.clone());
                    }
                    break;
                }
                cur = reg.parent.clone();
            } else {
                break;
            }
        }
        let params = match ancestor_params {
            Some(p) => p,
            None => return,
        };
        let ps = Self::emit_params_vec(&params);
        let args = params
            .iter()
            .map(|p| match p {
                Param::Named { name, .. } => name.clone(),
                Param::Vararg => "...".to_string(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        self.blank();
        self.line(&format!("function {name}.new({ps})"));
        self.indented(|s| {
            s.line(&format!("local self = {parent}.new({args})"));
            s.line(&format!("setmetatable(self, {name})"));
            for (_, f) in fields {
                let val = f
                    .value
                    .as_ref()
                    .map(|v| s.emit_expr(v))
                    .unwrap_or_else(|| "nil".to_string());
                s.line(&format!("self.{} = {val}", f.name));
            }
            s.line("return self");
        });
        self.line("end");
    }

    fn emit_params_vec(params: &[Param]) -> String {
        params
            .iter()
            .map(|p| match p {
                Param::Named { name, .. } => name.clone(),
                Param::Vararg => "...".to_string(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn emit_params_ref(params: &[&Param]) -> String {
        params
            .iter()
            .map(|p| match p {
                Param::Named { name, .. } => name.clone(),
                Param::Vararg => "...".to_string(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    // ─── Expressions ──────────────────────────────────────────────────────────

    fn lower_expr_sequence(&mut self, expressions: &[&Expr]) -> (Vec<String>, Vec<String>) {
        let lowered = expressions
            .iter()
            .map(|expr| self.lower_expr(expr))
            .collect::<Vec<_>>();
        let materialize =
            expressions.len() > 1 && lowered.iter().any(|item| !item.prelude.is_empty());
        let mut prelude = Vec::new();
        let mut values = Vec::new();
        for item in lowered {
            prelude.extend(item.prelude);
            if materialize {
                let name = self.generated_name("expr");
                prelude.push(self.current_line(&format!("local {name} = {}", item.expr)));
                values.push(name);
            } else {
                values.push(item.expr);
            }
        }
        (prelude, values)
    }

    fn lower_if_expr(&mut self, if_expr: &IfExpr) -> LoweredExpr {
        // Luau can keep the compact form when no branch has statements.  We
        // deliberately keep nested IfExpr out of this fast path so a nested
        // lowering never needs to move code across the surrounding branch.
        if self.target == Target::Luau && is_simple_native_if(if_expr) {
            let mut pieces = Vec::new();
            for (index, clause) in if_expr.clauses.iter().enumerate() {
                let condition = self.lower_expr(&clause.cond);
                let result = self.lower_expr(&clause.branch.result);
                let keyword = if index == 0 { "if" } else { "elseif" };
                pieces.push(format!("{keyword} {} then {}", condition.expr, result.expr));
            }
            let else_result = self.lower_expr(&if_expr.else_branch.result);
            pieces.push(format!("else {}", else_result.expr));
            return LoweredExpr {
                prelude: Vec::new(),
                expr: pieces.join(" "),
            };
        }

        // A block-valued expression is lowered to a statement sequence.  The
        // capture is important: when the expression occurs in an argument or
        // binary expression, its statements must be returned to the enclosing
        // statement before the final expression is emitted.
        let saved = std::mem::take(&mut self.out);
        self.out = Vec::new();
        let result_name = self.generated_name("if");
        self.line(&format!("local {result_name}"));
        self.emit_if_expr_chain(if_expr, 0, &result_name);
        let prelude = std::mem::replace(&mut self.out, saved);
        LoweredExpr {
            prelude,
            expr: result_name,
        }
    }

    fn emit_if_expr_chain(&mut self, if_expr: &IfExpr, index: usize, result_name: &str) {
        let clause = &if_expr.clauses[index];
        if let Expr::Bind { name, value, .. } = &clause.cond {
            self.line("do");
            self.indented(|this| {
                let value = this.emit_expr(value);
                this.line(&format!("local {name} = {value}"));
                this.line(&format!("if {name} then"));
                this.indented(|this| this.emit_if_expr_branch(&clause.branch, result_name));
                this.line("else");
                this.indented(|this| {
                    if index + 1 < if_expr.clauses.len() {
                        this.emit_if_expr_chain(if_expr, index + 1, result_name);
                    } else {
                        this.emit_if_expr_branch(&if_expr.else_branch, result_name);
                    }
                });
                this.line("end");
            });
            self.line("end");
            return;
        }
        let condition = self.lower_expr(&clause.cond);
        self.append_raw(condition.prelude);
        self.line(&format!(
            "if {} then",
            parenthesize_if_expr(&clause.cond, &condition.expr)
        ));
        self.indented(|this| this.emit_if_expr_branch(&clause.branch, result_name));
        self.line("else");
        self.indented(|this| {
            if index + 1 < if_expr.clauses.len() {
                this.emit_if_expr_chain(if_expr, index + 1, result_name);
            } else {
                this.emit_if_expr_branch(&if_expr.else_branch, result_name);
            }
        });
        self.line("end");
    }

    fn emit_if_expr_branch(&mut self, branch: &IfExprBranch, result_name: &str) {
        self.push_scope();
        for statement in &branch.statements {
            self.emit_stmt(statement);
            self.emit_dispatch_guard();
        }
        let result = self.lower_expr(&branch.result);
        self.append_raw(result.prelude);
        self.line(&format!("{result_name} = {}", result.expr));
        self.pop_scope();
    }

    fn lower_expr(&mut self, expr: &Expr) -> LoweredExpr {
        match expr {
            Expr::Nil => LoweredExpr {
                prelude: Vec::new(),
                expr: "nil".to_string(),
            },
            Expr::True => LoweredExpr {
                prelude: Vec::new(),
                expr: "true".to_string(),
            },
            Expr::False => LoweredExpr {
                prelude: Vec::new(),
                expr: "false".to_string(),
            },
            Expr::Number(v) => LoweredExpr {
                prelude: Vec::new(),
                expr: v.clone(),
            },
            Expr::Str(v) => LoweredExpr {
                prelude: Vec::new(),
                expr: format!("\"{}\"", Self::escape_string(v)),
            },
            Expr::InterpolatedString(parts) => self.lower_interpolated(parts),
            Expr::Vararg => LoweredExpr {
                prelude: Vec::new(),
                expr: "...".to_string(),
            },
            Expr::Ident { name: n, .. } => LoweredExpr {
                prelude: Vec::new(),
                expr: n.clone(),
            },
            Expr::SelfExpr => LoweredExpr {
                prelude: Vec::new(),
                expr: "self".to_string(),
            },
            Expr::SuperExpr => LoweredExpr {
                prelude: Vec::new(),
                expr: self.parent_name().unwrap_or_else(|| "nil".to_string()),
            },
            Expr::Field { obj, name } => {
                if matches!(obj.as_ref(), Expr::SuperExpr) {
                    let parent = self.parent_name().unwrap_or_else(|| "nil".to_string());
                    return LoweredExpr {
                        prelude: Vec::new(),
                        expr: format!("{parent}.{name}"),
                    };
                }
                let (prelude, values) = self.lower_expr_sequence(&[obj.as_ref()]);
                LoweredExpr {
                    prelude,
                    expr: format!("{}.{name}", parenthesize_if_expr(obj, &values[0])),
                }
            }
            Expr::Index { obj, key } => {
                let (prelude, values) = self.lower_expr_sequence(&[obj.as_ref(), key.as_ref()]);
                LoweredExpr {
                    prelude,
                    expr: format!(
                        "{}[{}]",
                        parenthesize_if_expr(obj, &values[0]),
                        parenthesize_if_expr(key, &values[1])
                    ),
                }
            }
            Expr::Call { callee, args } => {
                let mut children = vec![callee.as_ref()];
                children.extend(args.iter());
                let (prelude, values) = self.lower_expr_sequence(&children);
                let callee_str = parenthesize_if_expr(callee, &values[0]);
                let args_str = args
                    .iter()
                    .zip(&values[1..])
                    .map(|(arg, value)| parenthesize_if_expr(arg, value))
                    .collect::<Vec<_>>()
                    .join(", ");
                if let Expr::Field { obj, name } = callee.as_ref() {
                    // super.method(args) → ParentName.method(self, args)
                    if matches!(obj.as_ref(), Expr::SuperExpr) {
                        let parent = self.parent_name().unwrap_or_else(|| "nil".to_string());
                        let all_args = if args_str.is_empty() {
                            "self".to_string()
                        } else {
                            format!("self, {args_str}")
                        };
                        return LoweredExpr {
                            prelude,
                            expr: format!("{parent}.{name}({all_args})"),
                        };
                    }
                    let receiver = self.lower_expr(obj).expr;
                    if let Expr::Ident {
                        name: class_name, ..
                    } = obj.as_ref()
                    {
                        if self.registry.contains_key(class_name) {
                            if let Some(call) =
                                self.private_method_call(class_name, name, &receiver, &args_str)
                            {
                                return LoweredExpr {
                                    prelude,
                                    expr: call,
                                };
                            }
                        }
                    }
                    // dot→colon for instance method calls
                    let obj_type = self.resolve_type(obj);
                    if let Some(class_name) = obj_type {
                        if let Some(call) =
                            self.private_method_call(&class_name, name, &receiver, &args_str)
                        {
                            return LoweredExpr {
                                prelude,
                                expr: call,
                            };
                        }
                        if let Some(method) = self.lookup_method(&class_name, name) {
                            if matches!(method.kind, MethodKind::Instance) {
                                return LoweredExpr {
                                    prelude,
                                    expr: format!("{receiver}:{name}({args_str})"),
                                };
                            }
                        }
                    }
                }
                LoweredExpr {
                    prelude,
                    expr: format!("{callee_str}({args_str})"),
                }
            }
            Expr::MethodCall { obj, method, args } => {
                let mut children = vec![obj.as_ref()];
                children.extend(args.iter());
                let (prelude, values) = self.lower_expr_sequence(&children);
                let o = parenthesize_if_expr(obj, &values[0]);
                let args_str = args
                    .iter()
                    .zip(&values[1..])
                    .map(|(arg, value)| parenthesize_if_expr(arg, value))
                    .collect::<Vec<_>>()
                    .join(", ");
                if let Some(class_name) = self.resolve_type(obj) {
                    if let Some(call) = self.private_method_call(&class_name, method, &o, &args_str)
                    {
                        return LoweredExpr {
                            prelude,
                            expr: call,
                        };
                    }
                }
                LoweredExpr {
                    prelude,
                    expr: format!("{o}:{method}({args_str})"),
                }
            }
            Expr::Unop { op, expr } => {
                let (prelude, values) = self.lower_expr_sequence(&[expr.as_ref()]);
                LoweredExpr {
                    prelude,
                    expr: format!("{op} {}", parenthesize_if_expr(expr, &values[0])),
                }
            }
            Expr::Binop {
                op, left, right, ..
            } => {
                let (prelude, values) = self.lower_expr_sequence(&[left.as_ref(), right.as_ref()]);
                LoweredExpr {
                    prelude,
                    expr: format!(
                        "{} {op} {}",
                        parenthesize_if_expr(left, &values[0]),
                        parenthesize_if_expr(right, &values[1])
                    ),
                }
            }
            Expr::Table(fields) => {
                if fields.is_empty() {
                    return LoweredExpr {
                        prelude: Vec::new(),
                        expr: "{}".to_string(),
                    };
                }
                let mut children = Vec::new();
                for field in fields {
                    match field {
                        TableField::Index { key, value } => {
                            children.push(key);
                            children.push(value);
                        }
                        TableField::Name { value, .. } | TableField::Value(value) => {
                            children.push(value);
                        }
                    }
                }
                let (prelude, values) = self.lower_expr_sequence(&children);
                let mut value_index = 0;
                let mut parts = Vec::new();
                for field in fields {
                    match field {
                        TableField::Index { .. } => {
                            let key = &values[value_index];
                            let value = &values[value_index + 1];
                            value_index += 2;
                            parts.push(format!("[{key}] = {value}"));
                        }
                        TableField::Name { name, .. } => {
                            parts.push(format!("{name} = {}", values[value_index]));
                            value_index += 1;
                        }
                        TableField::Value(_) => {
                            parts.push(values[value_index].clone());
                            value_index += 1;
                        }
                    }
                }
                LoweredExpr {
                    prelude,
                    expr: format!("{{ {} }}", parts.join(", ")),
                }
            }
            Expr::Function { params, body, .. } => {
                // inline function: capture output at current indent
                let saved = std::mem::take(&mut self.out);
                self.indent += 1;
                self.push_scope();
                self.emit_function_body(body);
                self.pop_scope();
                let body_lines = std::mem::replace(&mut self.out, saved);
                self.indent -= 1;
                let body_str = body_lines.join("\n");
                let ps = Self::emit_params_vec(params);
                LoweredExpr {
                    prelude: Vec::new(),
                    expr: format!(
                        "function({ps})\n{body_str}\n{}end",
                        "    ".repeat(self.indent)
                    ),
                }
            }
            Expr::If(if_expr) => self.lower_if_expr(if_expr),
            Expr::Bind { name, .. } => LoweredExpr {
                prelude: Vec::new(),
                expr: name.clone(),
            },
        }
    }

    fn lower_interpolated(&mut self, parts: &[InterpolatedPart]) -> LoweredExpr {
        let expressions = parts
            .iter()
            .filter_map(|part| match part {
                InterpolatedPart::Expr(expr) => Some(expr),
                InterpolatedPart::Literal(_) => None,
            })
            .collect::<Vec<_>>();
        let (prelude, values) = self.lower_expr_sequence(&expressions);
        let mut value_index = 0;
        if self.target == Target::Luau {
            let mut output = String::from("`");
            for part in parts {
                match part {
                    InterpolatedPart::Literal(value) => output.push_str(
                        &value
                            .replace('\\', "\\\\")
                            .replace('`', "\\`")
                            .replace('{', "\\{")
                            .replace('}', "\\}"),
                    ),
                    InterpolatedPart::Expr(_) => {
                        output.push('{');
                        output.push_str(&values[value_index]);
                        output.push('}');
                        value_index += 1;
                    }
                }
            }
            output.push('`');
            return LoweredExpr {
                prelude,
                expr: output,
            };
        }

        let mut parts_out = Vec::new();
        for part in parts {
            match part {
                InterpolatedPart::Literal(value) if !value.is_empty() => {
                    parts_out.push(format!("\"{}\"", Self::escape_string(value)));
                }
                InterpolatedPart::Expr(_) => {
                    parts_out.push(format!("tostring({})", values[value_index]));
                    value_index += 1;
                }
                InterpolatedPart::Literal(_) => {}
            }
        }
        LoweredExpr {
            prelude,
            expr: if parts_out.is_empty() {
                "\"\"".to_string()
            } else {
                format!("({})", parts_out.join(" .. "))
            },
        }
    }

    fn emit_expr(&mut self, expr: &Expr) -> String {
        let lowered = self.lower_expr(expr);
        self.append_raw(lowered.prelude);
        lowered.expr
    }
}

fn is_simple_native_if(if_expr: &IfExpr) -> bool {
    if_expr.clauses.iter().all(|clause| {
        clause.branch.statements.is_empty()
            && !matches!(&clause.cond, Expr::Bind { .. })
            && !contains_if_expr(&clause.cond)
            && !contains_if_expr(&clause.branch.result)
    }) && if_expr.else_branch.statements.is_empty()
        && !contains_if_expr(&if_expr.else_branch.result)
}

fn parenthesize_if_expr(original: &Expr, rendered: &str) -> String {
    if matches!(original, Expr::If(_)) {
        format!("({rendered})")
    } else {
        rendered.to_string()
    }
}

fn contains_if_expr(expr: &Expr) -> bool {
    match expr {
        Expr::If(_) => true,
        Expr::Field { obj, .. } | Expr::Unop { expr: obj, .. } => contains_if_expr(obj),
        Expr::Index { obj, key } => contains_if_expr(obj) || contains_if_expr(key),
        Expr::Call { callee, args } => {
            contains_if_expr(callee) || args.iter().any(contains_if_expr)
        }
        Expr::MethodCall { obj, args, .. } => {
            contains_if_expr(obj) || args.iter().any(contains_if_expr)
        }
        Expr::Binop { left, right, .. } => contains_if_expr(left) || contains_if_expr(right),
        Expr::Table(fields) => fields.iter().any(|field| match field {
            TableField::Index { key, value } => contains_if_expr(key) || contains_if_expr(value),
            TableField::Name { value, .. } | TableField::Value(value) => contains_if_expr(value),
        }),
        Expr::Function { body, .. } => body.iter().any(stmt_contains_if_expr),
        Expr::InterpolatedString(parts) => parts.iter().any(|part| match part {
            InterpolatedPart::Literal(_) => false,
            InterpolatedPart::Expr(expr) => contains_if_expr(expr),
        }),
        Expr::Bind { value, .. } => contains_if_expr(value),
        Expr::Nil
        | Expr::True
        | Expr::False
        | Expr::Number(_)
        | Expr::Str(_)
        | Expr::Vararg
        | Expr::Ident { .. }
        | Expr::SelfExpr
        | Expr::SuperExpr => false,
    }
}

fn stmt_contains_if_expr(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Local { values, .. } | Stmt::Const { values, .. } => {
            values.iter().any(contains_if_expr)
        }
        Stmt::FunctionDecl { body, .. }
        | Stmt::Do { body }
        | Stmt::While { body, .. }
        | Stmt::Repeat { body, .. }
        | Stmt::NumericFor { body, .. }
        | Stmt::GenericFor { body, .. } => body.iter().any(stmt_contains_if_expr),
        Stmt::Assign { targets, values } => targets.iter().chain(values).any(contains_if_expr),
        Stmt::If { clauses, else_body } => {
            clauses.iter().any(|clause| {
                contains_if_expr(&clause.cond) || clause.body.iter().any(stmt_contains_if_expr)
            }) || else_body
                .as_deref()
                .is_some_and(|body| body.iter().any(stmt_contains_if_expr))
        }
        Stmt::Return(values) => values.iter().any(contains_if_expr),
        Stmt::ExprStmt(expr) => contains_if_expr(expr),
        Stmt::ClassDecl(class) => class
            .top_level_members
            .iter()
            .chain(class.blocks.iter().flat_map(|block| block.members.iter()))
            .any(|member| match member {
                Member::Field(field) => field.value.as_ref().is_some_and(contains_if_expr),
                Member::Method(method) => method
                    .body
                    .as_deref()
                    .is_some_and(|body| body.iter().any(stmt_contains_if_expr)),
            }),
        _ => false,
    }
}

fn collect_program_names(program: &Program) -> HashSet<String> {
    let mut names = HashSet::new();
    for stmt in &program.stmts {
        collect_stmt_names(stmt, &mut names);
    }
    names
}

fn collect_stmt_names(stmt: &Stmt, names: &mut HashSet<String>) {
    match stmt {
        Stmt::Local {
            names: bindings,
            values,
            ..
        }
        | Stmt::Const {
            names: bindings,
            values,
            ..
        } => {
            names.extend(bindings.iter().cloned());
            for value in values {
                collect_expr_names(value, names);
            }
        }
        Stmt::FunctionDecl {
            name, params, body, ..
        } => {
            names.insert(name.clone());
            collect_param_names(params, names);
            for statement in body {
                collect_stmt_names(statement, names);
            }
        }
        Stmt::Assign { targets, values } => {
            for expr in targets.iter().chain(values) {
                collect_expr_names(expr, names);
            }
        }
        Stmt::Do { body: _ }
        | Stmt::While { body: _, .. }
        | Stmt::Repeat { body: _, .. }
        | Stmt::NumericFor { body: _, .. }
        | Stmt::GenericFor { body: _, .. } => {
            match stmt {
                Stmt::NumericFor { name, .. } => {
                    names.insert(name.clone());
                }
                Stmt::GenericFor {
                    names: bindings, ..
                } => {
                    names.extend(bindings.iter().cloned());
                }
                _ => {}
            }
            match stmt {
                Stmt::While { cond, .. } | Stmt::Repeat { cond, .. } => {
                    collect_expr_names(cond, names)
                }
                Stmt::NumericFor {
                    start, limit, step, ..
                } => {
                    collect_expr_names(start, names);
                    collect_expr_names(limit, names);
                    if let Some(step) = step {
                        collect_expr_names(step, names);
                    }
                }
                Stmt::GenericFor { iters, .. } => {
                    for iter in iters {
                        collect_expr_names(iter, names);
                    }
                }
                _ => {}
            }
            if let Stmt::Do { body }
            | Stmt::While { body, .. }
            | Stmt::Repeat { body, .. }
            | Stmt::NumericFor { body, .. }
            | Stmt::GenericFor { body, .. } = stmt
            {
                for statement in body {
                    collect_stmt_names(statement, names);
                }
            }
        }
        Stmt::If { clauses, else_body } => {
            for clause in clauses {
                collect_expr_names(&clause.cond, names);
                for statement in &clause.body {
                    collect_stmt_names(statement, names);
                }
            }
            if let Some(body) = else_body {
                for statement in body {
                    collect_stmt_names(statement, names);
                }
            }
        }
        Stmt::Return(values) => {
            for value in values {
                collect_expr_names(value, names);
            }
        }
        Stmt::Goto { label, .. } | Stmt::Label { name: label, .. } => {
            names.insert(label.clone());
        }
        Stmt::ExprStmt(expr) => collect_expr_names(expr, names),
        Stmt::ClassDecl(class) => {
            names.insert(class.name.clone());
            for member in class
                .top_level_members
                .iter()
                .chain(class.blocks.iter().flat_map(|block| block.members.iter()))
            {
                match member {
                    Member::Field(field) => {
                        names.insert(field.name.clone());
                        if let Some(value) = &field.value {
                            collect_expr_names(value, names);
                        }
                    }
                    Member::Method(method) => {
                        names.insert(method.name.clone());
                        collect_param_names(&method.params, names);
                        if let Some(body) = &method.body {
                            for statement in body {
                                collect_stmt_names(statement, names);
                            }
                        }
                    }
                }
            }
        }
        Stmt::ImportDecl { module_name } => {
            names.insert(module_name.clone());
        }
        Stmt::DeclareStmt { name, .. } => {
            names.insert(name.clone());
        }
        Stmt::Break | Stmt::Continue | Stmt::RawLua54(_) => {}
    }
}

fn collect_param_names(params: &[Param], names: &mut HashSet<String>) {
    for param in params {
        if let Param::Named { name, .. } = param {
            names.insert(name.clone());
        }
    }
}

fn collect_expr_names(expr: &Expr, names: &mut HashSet<String>) {
    match expr {
        Expr::Ident { name, .. } => {
            names.insert(name.clone());
        }
        Expr::Field { obj, name } => {
            names.insert(name.clone());
            collect_expr_names(obj, names);
        }
        Expr::Index { obj, key } => {
            collect_expr_names(obj, names);
            collect_expr_names(key, names);
        }
        Expr::Call { callee, args } => {
            collect_expr_names(callee, names);
            for arg in args {
                collect_expr_names(arg, names);
            }
        }
        Expr::MethodCall { obj, method, args } => {
            names.insert(method.clone());
            collect_expr_names(obj, names);
            for arg in args {
                collect_expr_names(arg, names);
            }
        }
        Expr::Unop { expr, .. } => collect_expr_names(expr, names),
        Expr::Binop { left, right, .. } => {
            collect_expr_names(left, names);
            collect_expr_names(right, names);
        }
        Expr::Table(fields) => {
            for field in fields {
                match field {
                    TableField::Index { key, value } => {
                        collect_expr_names(key, names);
                        collect_expr_names(value, names);
                    }
                    TableField::Name { name, value } => {
                        names.insert(name.clone());
                        collect_expr_names(value, names);
                    }
                    TableField::Value(value) => collect_expr_names(value, names),
                }
            }
        }
        Expr::Function { params, body, .. } => {
            collect_param_names(params, names);
            for statement in body {
                collect_stmt_names(statement, names);
            }
        }
        Expr::InterpolatedString(parts) => {
            for part in parts {
                if let InterpolatedPart::Expr(expr) = part {
                    collect_expr_names(expr, names);
                }
            }
        }
        Expr::If(if_expr) => {
            for clause in &if_expr.clauses {
                collect_expr_names(&clause.cond, names);
                for statement in &clause.branch.statements {
                    collect_stmt_names(statement, names);
                }
                collect_expr_names(&clause.branch.result, names);
            }
            for statement in &if_expr.else_branch.statements {
                collect_stmt_names(statement, names);
            }
            collect_expr_names(&if_expr.else_branch.result, names);
        }
        Expr::Bind { name, value, .. } => {
            names.insert(name.clone());
            collect_expr_names(value, names);
        }
        Expr::Nil
        | Expr::True
        | Expr::False
        | Expr::Number(_)
        | Expr::Str(_)
        | Expr::Vararg
        | Expr::SelfExpr
        | Expr::SuperExpr => {}
    }
}

fn contains_goto(body: &[Stmt]) -> bool {
    body.iter().any(|stmt| match stmt {
        Stmt::Goto { .. } | Stmt::Label { .. } => true,
        Stmt::Do { body }
        | Stmt::While { body, .. }
        | Stmt::Repeat { body, .. }
        | Stmt::NumericFor { body, .. }
        | Stmt::GenericFor { body, .. } => contains_goto(body),
        Stmt::If { clauses, else_body } => {
            clauses
                .iter()
                .any(|clause| contains_goto_expr(&clause.cond) || contains_goto(&clause.body))
                || else_body.as_deref().is_some_and(contains_goto)
        }
        Stmt::Local { values, .. } | Stmt::Const { values, .. } | Stmt::Return(values) => {
            values.iter().any(contains_goto_expr)
        }
        Stmt::Assign { targets, values } => targets.iter().chain(values).any(contains_goto_expr),
        Stmt::ExprStmt(expr) => contains_goto_expr(expr),
        Stmt::FunctionDecl { .. } | Stmt::ClassDecl(_) => false,
        _ => false,
    })
}

fn contains_goto_expr(expr: &Expr) -> bool {
    match expr {
        Expr::If(if_expr) => {
            if_expr.clauses.iter().any(|clause| {
                contains_goto_expr(&clause.cond)
                    || contains_goto(&clause.branch.statements)
                    || contains_goto_expr(&clause.branch.result)
            }) || contains_goto(&if_expr.else_branch.statements)
                || contains_goto_expr(&if_expr.else_branch.result)
        }
        Expr::Function { .. } => false,
        Expr::Field { obj, .. } | Expr::Unop { expr: obj, .. } => contains_goto_expr(obj),
        Expr::Index { obj, key } => contains_goto_expr(obj) || contains_goto_expr(key),
        Expr::Call { callee, args } => {
            contains_goto_expr(callee) || args.iter().any(contains_goto_expr)
        }
        Expr::MethodCall { obj, args, .. } => {
            contains_goto_expr(obj) || args.iter().any(contains_goto_expr)
        }
        Expr::Binop { left, right, .. } => contains_goto_expr(left) || contains_goto_expr(right),
        Expr::Table(fields) => fields.iter().any(|field| match field {
            TableField::Index { key, value } => {
                contains_goto_expr(key) || contains_goto_expr(value)
            }
            TableField::Name { value, .. } | TableField::Value(value) => contains_goto_expr(value),
        }),
        Expr::InterpolatedString(parts) => parts.iter().any(|part| match part {
            InterpolatedPart::Literal(_) => false,
            InterpolatedPart::Expr(expr) => contains_goto_expr(expr),
        }),
        _ => false,
    }
}

fn contains_continue(body: &[Stmt]) -> bool {
    body.iter().any(|stmt| match stmt {
        Stmt::Continue => true,
        Stmt::Do { body } => contains_continue(body),
        Stmt::If { clauses, else_body } => {
            clauses.iter().any(|clause| {
                contains_continue_expr(&clause.cond) || contains_continue(&clause.body)
            }) || else_body.as_deref().is_some_and(contains_continue)
        }
        Stmt::Local { values, .. } | Stmt::Const { values, .. } | Stmt::Return(values) => {
            values.iter().any(contains_continue_expr)
        }
        Stmt::Assign { targets, values } => {
            targets.iter().chain(values).any(contains_continue_expr)
        }
        Stmt::ExprStmt(expr) => contains_continue_expr(expr),
        // A continue in a nested loop belongs to that loop.
        Stmt::While { .. }
        | Stmt::Repeat { .. }
        | Stmt::NumericFor { .. }
        | Stmt::GenericFor { .. }
        | Stmt::FunctionDecl { .. }
        | Stmt::ClassDecl(_) => false,
        _ => false,
    })
}

fn contains_continue_expr(expr: &Expr) -> bool {
    match expr {
        Expr::If(if_expr) => {
            if_expr.clauses.iter().any(|clause| {
                contains_continue_expr(&clause.cond)
                    || contains_continue(&clause.branch.statements)
                    || contains_continue_expr(&clause.branch.result)
            }) || contains_continue(&if_expr.else_branch.statements)
                || contains_continue_expr(&if_expr.else_branch.result)
        }
        Expr::Function { .. } => false,
        Expr::Field { obj, .. } | Expr::Unop { expr: obj, .. } => contains_continue_expr(obj),
        Expr::Index { obj, key } => contains_continue_expr(obj) || contains_continue_expr(key),
        Expr::Call { callee, args } => {
            contains_continue_expr(callee) || args.iter().any(contains_continue_expr)
        }
        Expr::MethodCall { obj, args, .. } => {
            contains_continue_expr(obj) || args.iter().any(contains_continue_expr)
        }
        Expr::Binop { left, right, .. } => {
            contains_continue_expr(left) || contains_continue_expr(right)
        }
        Expr::Table(fields) => fields.iter().any(|field| match field {
            TableField::Index { key, value } => {
                contains_continue_expr(key) || contains_continue_expr(value)
            }
            TableField::Name { value, .. } | TableField::Value(value) => {
                contains_continue_expr(value)
            }
        }),
        Expr::InterpolatedString(parts) => parts.iter().any(|part| match part {
            InterpolatedPart::Literal(_) => false,
            InterpolatedPart::Expr(expr) => contains_continue_expr(expr),
        }),
        _ => false,
    }
}
