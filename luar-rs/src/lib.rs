pub mod ast;
pub mod checker;
pub mod codegen;
pub mod control_flow;
pub mod include;
pub mod lexer;
pub mod modules;
pub mod parser;
pub mod resolver;

use crate::lexer::SourceSpan;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::path::Path;
use std::path::PathBuf;
use std::ptr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Target {
    #[default]
    Luau,
    Lua54,
}

impl std::str::FromStr for Target {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "luau" => Ok(Self::Luau),
            "lua54" => Ok(Self::Lua54),
            _ => Err(format!(
                "unknown target '{value}'; expected 'luau' or 'lua54'"
            )),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CompileOptions {
    pub target: Target,
    pub source_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiagnosticReport {
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone)]
pub struct Analysis {
    pub program: ast::Program,
    pub diagnostics: Vec<Diagnostic>,
}

// Thread-local error buffer for luar_get_errors
thread_local! {
    static LAST_ERRORS: RefCell<String> = RefCell::new(String::new());
}

/// Compile Luar source to Luau source.
///
/// Returns 0 on success, -1 on error.
/// On success, `out_buf` is filled with the null-terminated Luau source.
/// On error, call `luar_get_errors` to retrieve error messages.
#[unsafe(no_mangle)]
pub extern "C" fn luar_compile(src: *const c_char, out_buf: *mut c_char, out_len: usize) -> c_int {
    if src.is_null() || out_buf.is_null() || out_len == 0 {
        return -1;
    }

    let source = unsafe {
        match CStr::from_ptr(src).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => {
                store_error("invalid UTF-8 in source");
                return -1;
            }
        }
    };

    let output = match compile_source(&source, None) {
        Ok(output) => output,
        Err(errors) => {
            store_error(&errors.join("\n"));
            return -1;
        }
    };

    write_output(output, out_buf, out_len)
}

/// Compile Luar source with its file path available for adjacent `.luard` lookup.
///
/// The argument order is kept C-friendly and matches `luar_compile`, with
/// `source_path` inserted immediately after `src`.
#[unsafe(no_mangle)]
pub extern "C" fn luar_compile_with_path(
    src: *const c_char,
    source_path: *const c_char,
    out_buf: *mut c_char,
    out_len: usize,
) -> c_int {
    if src.is_null() || source_path.is_null() || out_buf.is_null() || out_len == 0 {
        return -1;
    }

    let source = unsafe {
        match CStr::from_ptr(src).to_str() {
            Ok(source) => source,
            Err(_) => {
                store_error("invalid UTF-8 in source");
                return -1;
            }
        }
    };
    let source_path = unsafe {
        match CStr::from_ptr(source_path).to_str() {
            Ok(path) => path,
            Err(_) => {
                store_error("invalid UTF-8 in source path");
                return -1;
            }
        }
    };
    let output = match compile_source(source, Some(Path::new(source_path))) {
        Ok(output) => output,
        Err(errors) => {
            store_error(&errors.join("\n"));
            return -1;
        }
    };

    write_output(output, out_buf, out_len)
}

/// Compile Luar source for an explicit target (`"luau"` or `"lua54"`).
/// `source_path` may be null when the source has no imports or includes.
/// Existing C entry points remain Luau-default compatibility wrappers.
#[unsafe(no_mangle)]
pub extern "C" fn luar_compile_with_target(
    src: *const c_char,
    source_path: *const c_char,
    target: *const c_char,
    out_buf: *mut c_char,
    out_len: usize,
) -> c_int {
    if src.is_null() || target.is_null() || out_buf.is_null() || out_len == 0 {
        return -1;
    }
    let source = unsafe {
        match CStr::from_ptr(src).to_str() {
            Ok(source) => source,
            Err(_) => {
                store_error("invalid UTF-8 in source");
                return -1;
            }
        }
    };
    let target = unsafe {
        match CStr::from_ptr(target)
            .to_str()
            .ok()
            .and_then(|value| value.parse().ok())
        {
            Some(target) => target,
            None => {
                store_error("invalid target; expected 'luau' or 'lua54'");
                return -1;
            }
        }
    };
    let source_path = if source_path.is_null() {
        None
    } else {
        let source_path = unsafe {
            match CStr::from_ptr(source_path).to_str() {
                Ok(path) => path,
                Err(_) => {
                    store_error("invalid UTF-8 in source path");
                    return -1;
                }
            }
        };
        Some(PathBuf::from(source_path))
    };
    let options = CompileOptions {
        target,
        source_path,
    };
    let output = match compile_source_with_options(source, &options) {
        Ok(output) => output,
        Err(errors) => {
            store_error(
                &errors
                    .iter()
                    .map(|error| format!("[{}] {}", error.line, error.message))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            return -1;
        }
    };
    write_output(output, out_buf, out_len)
}

/// Explicit-target form without a source path.  This is useful for embedded
/// callers that do not use imports/includes.
#[unsafe(no_mangle)]
pub extern "C" fn luar_compile_target(
    src: *const c_char,
    target: *const c_char,
    out_buf: *mut c_char,
    out_len: usize,
) -> c_int {
    luar_compile_with_target(src, ptr::null(), target, out_buf, out_len)
}

/// Explicit-target form with a required source path for module/include lookup.
#[unsafe(no_mangle)]
pub extern "C" fn luar_compile_with_path_target(
    src: *const c_char,
    source_path: *const c_char,
    target: *const c_char,
    out_buf: *mut c_char,
    out_len: usize,
) -> c_int {
    if source_path.is_null() {
        store_error("source path must not be null");
        return -1;
    }
    luar_compile_with_target(src, source_path, target, out_buf, out_len)
}

pub fn compile_source(source: &str, source_path: Option<&Path>) -> Result<String, Vec<String>> {
    let options = CompileOptions {
        target: Target::Luau,
        source_path: source_path.map(Path::to_path_buf),
    };
    compile_source_with_options(source, &options).map_err(|diagnostics| {
        diagnostics
            .into_iter()
            .map(|diagnostic| format!("[{}] {}", diagnostic.line, diagnostic.message))
            .collect()
    })
}

pub fn compile_source_with_options(
    source: &str,
    options: &CompileOptions,
) -> Result<String, Vec<Diagnostic>> {
    let analysis = analyze_source_with_options(source, options)?;
    Ok(codegen::Codegen::for_target(options.target).generate(&analysis.program))
}

pub fn check_source_with_options(
    source: &str,
    options: &CompileOptions,
) -> Result<ast::Program, Vec<Diagnostic>> {
    analyze_source_with_options(source, options).map(|analysis| analysis.program)
}

pub fn analyze_source_with_options(
    source: &str,
    options: &CompileOptions,
) -> Result<Analysis, Vec<Diagnostic>> {
    let file = options
        .source_path
        .as_deref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<stdin>".to_string());
    let expanded = include::expand_source(source, options.source_path.as_deref(), options.target)
        .map_err(|error| vec![diagnostic_from_message(&file, &error)])?;
    let mut parser = parser::Parser::new(&expanded.source).map_err(|error| {
        vec![diagnostic_from_expanded(
            &expanded,
            &file,
            error.span,
            Severity::Error,
            error.message,
        )]
    })?;
    let mut program = parser.parse().map_err(|error| {
        vec![diagnostic_from_expanded(
            &expanded,
            &file,
            error.span,
            Severity::Error,
            error.message,
        )]
    })?;
    convert_raw_lua_markers(&mut program.stmts);

    let mut imports = Vec::new();
    let mut seen_imports = std::collections::HashSet::new();
    let mut errors: Vec<Diagnostic> = Vec::new();
    for stmt in &program.stmts {
        if let ast::Stmt::ImportDecl { module_name } = stmt {
            if seen_imports.insert(module_name.clone()) {
                imports.push(module_name.clone());
            } else {
                errors.push(diagnostic(
                    &file,
                    1,
                    format!("module '{module_name}' is imported more than once"),
                ));
            }
        }
    }

    let mut definitions = Vec::new();
    if !imports.is_empty() {
        let Some(source_path) = options.source_path.as_deref() else {
            errors.push(diagnostic(
                &file,
                1,
                "import declarations require luar_compile_with_path and a source file path"
                    .to_string(),
            ));
            return Err(errors);
        };
        if source_path.as_os_str().is_empty() {
            errors.push(diagnostic(
                &file,
                1,
                "import declarations require a non-empty source file path".to_string(),
            ));
            return Err(errors);
        }
        for module_name in imports {
            match modules::load_definition(&module_name, source_path) {
                Ok(definition) => definitions.push(definition),
                Err(error) => errors.push(diagnostic(&file, error.line.max(1), error.message)),
            }
        }
    }

    let resolver_errors =
        resolver::Resolver::new(definitions, options.target).resolve(&mut program);
    errors.extend(resolver_errors.into_iter().map(|error| {
        diagnostic_from_expanded(&expanded, &file, error.span, error.severity, error.message)
    }));
    let checker_errors = checker::Checker::new().check(&mut program);
    errors.extend(checker_errors.into_iter().map(|error| {
        let span = error.span.unwrap_or(SourceSpan {
            line: error.line.max(1),
            column: 1,
            end_line: error.line.max(1),
            end_column: 2,
        });
        diagnostic_from_expanded_statement(&expanded, &file, span, Severity::Error, error.message)
    }));
    errors.extend(control_flow::validate(&program).into_iter().map(|error| {
        diagnostic_from_expanded_statement(
            &expanded,
            &file,
            SourceSpan {
                line: error.line.max(1),
                column: 1,
                end_line: error.line.max(1),
                end_column: 2,
            },
            Severity::Error,
            error.message,
        )
    }));
    if options.target == Target::Luau {
        errors.extend(validate_luau_label_layout(&program, &file));
    }
    if errors
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        return Err(errors);
    }

    Ok(Analysis {
        program,
        diagnostics: errors,
    })
}

pub fn dump_ir(source: &str, options: &CompileOptions) -> Result<String, Vec<Diagnostic>> {
    let program = check_source_with_options(source, options)?;
    Ok(control_flow::dump(&program))
}

fn diagnostic(file: &str, line: usize, message: String) -> Diagnostic {
    diagnostic_with_span(
        file,
        SourceSpan {
            line: line.max(1),
            column: 1,
            end_line: line.max(1),
            end_column: 2,
        },
        Severity::Error,
        message,
    )
}

fn diagnostic_with_span(
    file: &str,
    span: SourceSpan,
    severity: Severity,
    message: String,
) -> Diagnostic {
    Diagnostic {
        file: file.to_string(),
        line: span.line.max(1),
        column: span.column.max(1),
        end_line: span.end_line.max(1),
        end_column: span.end_column.max(span.column.saturating_add(1)),
        severity,
        message,
    }
}

fn diagnostic_from_expanded(
    expanded: &include::ExpandedSource,
    fallback_file: &str,
    span: SourceSpan,
    severity: Severity,
    message: String,
) -> Diagnostic {
    // A number of parser/control-flow errors only have a cursor-sized source
    // location.  Such a range is technically valid LSP, but a one-character
    // underline is difficult to associate with the statement that caused it.
    // Preserve precise token ranges and widen only point-like locations.
    let span = if severity == Severity::Error && is_point_like(span) {
        statement_span(&expanded.source, span)
    } else {
        span
    };
    let (file, span) = expanded.remap_span(span, fallback_file);
    diagnostic_with_span(&file, span, severity, message)
}

fn diagnostic_from_expanded_statement(
    expanded: &include::ExpandedSource,
    fallback_file: &str,
    span: SourceSpan,
    severity: Severity,
    message: String,
) -> Diagnostic {
    let span = statement_span(&expanded.source, span);
    let (file, span) = expanded.remap_span(span, fallback_file);
    diagnostic_with_span(&file, span, severity, message)
}

fn is_point_like(span: SourceSpan) -> bool {
    span.line == span.end_line && span.end_column <= span.column.saturating_add(1)
}

/// Expand a diagnostic to the non-whitespace contents of its source line.
/// Columns are counted as UTF-16 code units, matching both the lexer and LSP.
fn statement_span(source: &str, span: SourceSpan) -> SourceSpan {
    let Some(line) = source.lines().nth(span.line.saturating_sub(1)) else {
        return span;
    };
    let content = line.trim_end_matches([' ', '\t', '\r']);
    let Some(first) = content.find(|character: char| !character.is_whitespace()) else {
        return span;
    };
    let start_column = content[..first].encode_utf16().count() + 1;
    let end_column = content.encode_utf16().count() + 1;
    SourceSpan {
        line: span.line,
        column: start_column,
        end_line: span.line,
        end_column: end_column.max(start_column.saturating_add(1)),
    }
}

fn diagnostic_from_message(default_file: &str, message: &str) -> Diagnostic {
    let mut file = default_file.to_string();
    let mut line = 1;
    let mut text = message.to_string();
    if let Some(rest) = message.strip_prefix('[') {
        if let Some((number, tail)) = rest.split_once(']') {
            line = number.parse().unwrap_or(1).max(1);
            text = tail.trim_start().to_string();
        }
    } else {
        let parts = message.splitn(3, ':').collect::<Vec<_>>();
        if parts.len() == 3 {
            if let Ok(parsed) = parts[1].parse::<usize>() {
                file = parts[0].to_string();
                line = parsed.max(1);
                text = parts[2].trim_start().to_string();
            }
        }
    }
    diagnostic(&file, line, text)
}

fn convert_raw_lua_markers(stmts: &mut [ast::Stmt]) {
    use ast::{Expr, Member, Stmt};
    for stmt in stmts {
        let raw = match stmt {
            Stmt::ExprStmt(Expr::Call { callee, args })
                if matches!(callee.as_ref(), Expr::Ident { name, .. } if name == "__luar_raw_lua54")
                    && args.len() == 1 =>
            {
                match &args[0] {
                    Expr::Str(source) => Some(source.clone()),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(source) = raw {
            *stmt = Stmt::RawLua54(source);
            continue;
        }
        match stmt {
            Stmt::FunctionDecl { body, .. }
            | Stmt::Do { body }
            | Stmt::While { body, .. }
            | Stmt::Repeat { body, .. }
            | Stmt::NumericFor { body, .. }
            | Stmt::GenericFor { body, .. } => convert_raw_lua_markers(body),
            Stmt::If { clauses, else_body } => {
                for clause in clauses {
                    convert_raw_lua_markers(&mut clause.body);
                }
                if let Some(body) = else_body {
                    convert_raw_lua_markers(body);
                }
            }
            Stmt::ClassDecl(class) => {
                for member in class
                    .top_level_members
                    .iter_mut()
                    .chain(class.blocks.iter_mut().flat_map(|block| &mut block.members))
                {
                    if let Member::Method(method) = member {
                        if let Some(body) = &mut method.body {
                            convert_raw_lua_markers(body);
                        }
                    }
                }
            }
            Stmt::Local { values, .. }
            | Stmt::Const { values, .. }
            | Stmt::Assign { values, .. }
            | Stmt::Return(values) => {
                for value in values {
                    convert_raw_lua_markers_expr(value);
                }
            }
            Stmt::ExprStmt(expr) => convert_raw_lua_markers_expr(expr),
            _ => {}
        }
    }
}

fn convert_raw_lua_markers_expr(expr: &mut ast::Expr) {
    match expr {
        ast::Expr::If(if_expr) => {
            for clause in &mut if_expr.clauses {
                convert_raw_lua_markers(&mut clause.branch.statements);
                convert_raw_lua_markers_expr(&mut clause.cond);
                convert_raw_lua_markers_expr(&mut clause.branch.result);
            }
            convert_raw_lua_markers(&mut if_expr.else_branch.statements);
            convert_raw_lua_markers_expr(&mut if_expr.else_branch.result);
        }
        ast::Expr::Bind { value, .. } => convert_raw_lua_markers_expr(value),
        ast::Expr::Function { body, .. } => convert_raw_lua_markers(body),
        ast::Expr::InterpolatedString(parts) => {
            for part in parts {
                if let ast::InterpolatedPart::Expr(expr) = part {
                    convert_raw_lua_markers_expr(expr);
                }
            }
        }
        ast::Expr::Field { obj, .. } | ast::Expr::Unop { expr: obj, .. } => {
            convert_raw_lua_markers_expr(obj)
        }
        ast::Expr::Index { obj, key } => {
            convert_raw_lua_markers_expr(obj);
            convert_raw_lua_markers_expr(key);
        }
        ast::Expr::Call { callee, args } => {
            convert_raw_lua_markers_expr(callee);
            for arg in args {
                convert_raw_lua_markers_expr(arg);
            }
        }
        ast::Expr::MethodCall { obj, args, .. } => {
            convert_raw_lua_markers_expr(obj);
            for arg in args {
                convert_raw_lua_markers_expr(arg);
            }
        }
        ast::Expr::Binop { left, right, .. } => {
            convert_raw_lua_markers_expr(left);
            convert_raw_lua_markers_expr(right);
        }
        ast::Expr::Table(fields) => {
            for field in fields {
                match field {
                    ast::TableField::Index { key, value } => {
                        convert_raw_lua_markers_expr(key);
                        convert_raw_lua_markers_expr(value);
                    }
                    ast::TableField::Name { value, .. } | ast::TableField::Value(value) => {
                        convert_raw_lua_markers_expr(value)
                    }
                }
            }
        }
        _ => {}
    }
}

fn validate_luau_label_layout(program: &ast::Program, file: &str) -> Vec<Diagnostic> {
    fn walk_expr(expr: &ast::Expr, file: &str, errors: &mut Vec<Diagnostic>) {
        match expr {
            ast::Expr::Function { body, .. } => walk(body, false, file, errors),
            ast::Expr::If(if_expr) => {
                for clause in &if_expr.clauses {
                    walk_expr(&clause.cond, file, errors);
                    walk(&clause.branch.statements, true, file, errors);
                    walk_expr(&clause.branch.result, file, errors);
                }
                walk(&if_expr.else_branch.statements, true, file, errors);
                walk_expr(&if_expr.else_branch.result, file, errors);
            }
            ast::Expr::Bind { value, .. } => walk_expr(value, file, errors),
            ast::Expr::InterpolatedString(parts) => {
                for part in parts {
                    if let ast::InterpolatedPart::Expr(expr) = part {
                        walk_expr(expr, file, errors);
                    }
                }
            }
            ast::Expr::Field { obj, .. } | ast::Expr::Unop { expr: obj, .. } => {
                walk_expr(obj, file, errors)
            }
            ast::Expr::Index { obj, key } => {
                walk_expr(obj, file, errors);
                walk_expr(key, file, errors);
            }
            ast::Expr::Call { callee, args } => {
                walk_expr(callee, file, errors);
                for argument in args {
                    walk_expr(argument, file, errors);
                }
            }
            ast::Expr::MethodCall { obj, args, .. } => {
                walk_expr(obj, file, errors);
                for argument in args {
                    walk_expr(argument, file, errors);
                }
            }
            ast::Expr::Binop { left, right, .. } => {
                walk_expr(left, file, errors);
                walk_expr(right, file, errors);
            }
            ast::Expr::Table(fields) => {
                for field in fields {
                    match field {
                        ast::TableField::Index { key, value } => {
                            walk_expr(key, file, errors);
                            walk_expr(value, file, errors);
                        }
                        ast::TableField::Name { value, .. } | ast::TableField::Value(value) => {
                            walk_expr(value, file, errors)
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn walk(body: &[ast::Stmt], nested: bool, file: &str, errors: &mut Vec<Diagnostic>) {
        let mut saw_top_level_local = false;
        for stmt in body {
            match stmt {
                ast::Stmt::Local { .. } | ast::Stmt::Const { .. } if !nested => {
                    saw_top_level_local = true;
                }
                ast::Stmt::Label { line, name } if !nested && saw_top_level_local => errors.push(
                    diagnostic(
                        file,
                        *line,
                        format!(
                            "Luau target cannot preserve local scope across label '{}' yet; move the label before top-level local declarations",
                            name
                        ),
                    ),
                ),
                ast::Stmt::Label { line, .. } if nested => errors.push(diagnostic(
                    file,
                    *line,
                    "Luau target currently requires labels to be at function/chunk top level"
                        .to_string(),
                )),
                ast::Stmt::Do { body }
                | ast::Stmt::While { body, .. }
                | ast::Stmt::Repeat { body, .. }
                | ast::Stmt::NumericFor { body, .. }
                | ast::Stmt::GenericFor { body, .. } => walk(body, true, file, errors),
                ast::Stmt::If { clauses, else_body } => {
                    for clause in clauses {
                        walk(&clause.body, true, file, errors);
                    }
                    if let Some(body) = else_body {
                        walk(body, true, file, errors);
                    }
                }
                ast::Stmt::FunctionDecl { body, .. } => walk(body, false, file, errors),
                ast::Stmt::Local { values, .. } | ast::Stmt::Const { values, .. } => {
                    for value in values {
                        walk_expr(value, file, errors);
                    }
                }
                ast::Stmt::Assign { targets, values } => {
                    for expr in targets.iter().chain(values) {
                        walk_expr(expr, file, errors);
                    }
                }
                ast::Stmt::Return(values) => {
                    for value in values {
                        walk_expr(value, file, errors);
                    }
                }
                ast::Stmt::ExprStmt(expr) => walk_expr(expr, file, errors),
                ast::Stmt::ClassDecl(class) => {
                    for member in class
                        .top_level_members
                        .iter()
                        .chain(class.blocks.iter().flat_map(|block| &block.members))
                    {
                        match member {
                            ast::Member::Field(field) => {
                                if let Some(value) = &field.value {
                                    walk_expr(value, file, errors);
                                }
                            }
                            ast::Member::Method(method) => {
                                if let Some(body) = &method.body {
                                    walk(body, false, file, errors);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut errors = Vec::new();
    walk(&program.stmts, false, file, &mut errors);
    errors
}

fn write_output(output: String, out_buf: *mut c_char, out_len: usize) -> c_int {
    let c_output = match CString::new(output) {
        Ok(s) => s,
        Err(_) => {
            store_error("codegen output contains null byte");
            return -1;
        }
    };
    let bytes = c_output.as_bytes_with_nul();
    if bytes.len() > out_len {
        store_error("output buffer too small");
        return -1;
    }

    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf as *mut u8, bytes.len());
    }
    0
}

/// Retrieve the last error message(s) into `buf`.
/// Returns the number of bytes written (excluding null terminator), or -1 on failure.
#[unsafe(no_mangle)]
pub extern "C" fn luar_get_errors(buf: *mut c_char, buf_len: usize) -> c_int {
    if buf.is_null() || buf_len == 0 {
        return -1;
    }
    LAST_ERRORS.with(|e| {
        let s = e.borrow();
        let bytes = s.as_bytes();
        let n = bytes.len().min(buf_len - 1);
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, n);
            *buf.add(n) = 0;
        }
        n as c_int
    })
}

fn store_error(msg: &str) {
    LAST_ERRORS.with(|e| *e.borrow_mut() = msg.to_string());
}
