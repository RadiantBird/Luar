//! `!include` で貼り付けたモジュールの名前が、include元の名前とぶつかるとき、
//! include側の名前だけを別名へ書き換えるための字句レベルの補助。
//!
//! 文字列置換ではなくトークン単位で行い、変数・クラス名・型名の参照だけを対象にする。
//! メンバーアクセス(`x.name`)、メソッド呼び出し(`x:name()`)、テーブルのキー、
//! ラベル、クラスのメンバー宣言名は、名前が同じでも別物なので書き換えない。

use crate::ast::{Expr, IfExprBranch, InterpolatedPart, Member, Param, Stmt, TableField};
use crate::lexer::{Lexer, Token, TokenKind};
use crate::parser::Parser;
use std::collections::{HashMap, HashSet};

/// トップレベルで束縛される名前 (`local` / `const` / `function` / `class`)。
/// 構文解析できないソースでは `None`。
pub fn top_level_names(source: &str) -> Option<Vec<String>> {
    let program = Parser::new(source).ok()?.parse().ok()?;
    let mut names = Vec::new();
    for stmt in &program.stmts {
        match stmt {
            Stmt::Local { names: bound, .. } | Stmt::Const { names: bound, .. } => {
                names.extend(bound.iter().cloned());
            }
            Stmt::FunctionDecl { name, .. } if !name.contains('.') => names.push(name.clone()),
            Stmt::ClassDecl(decl) => names.push(decl.name.clone()),
            _ => {}
        }
    }
    Some(names)
}

/// ソース内のどこかで束縛される名前 (local/const/関数/クラス/引数/ループ変数/`:=`)。
/// 構文解析できないソースでは `None`。
pub fn bound_names(source: &str) -> Option<HashSet<String>> {
    let program = Parser::new(source).ok()?.parse().ok()?;
    let mut names = HashSet::new();
    collect_stmts(&program.stmts, &mut names);
    Some(names)
}

fn collect_stmts(stmts: &[Stmt], names: &mut HashSet<String>) {
    for stmt in stmts {
        collect_stmt(stmt, names);
    }
}

fn collect_params(params: &[Param], names: &mut HashSet<String>) {
    for param in params {
        if let Param::Named { name, .. } = param {
            names.insert(name.clone());
        }
    }
}

fn collect_stmt(stmt: &Stmt, names: &mut HashSet<String>) {
    match stmt {
        Stmt::Local { names: bound, values, .. } | Stmt::Const { names: bound, values, .. } => {
            names.extend(bound.iter().cloned());
            collect_exprs(values, names);
        }
        Stmt::FunctionDecl { name, params, body, .. } => {
            names.insert(name.split('.').next().unwrap_or(name).to_string());
            collect_params(params, names);
            collect_stmts(body, names);
        }
        Stmt::Assign { targets, values } => {
            collect_exprs(targets, names);
            collect_exprs(values, names);
        }
        Stmt::Do { body } => collect_stmts(body, names),
        Stmt::While { cond, body } | Stmt::Repeat { body, cond } => {
            collect_expr(cond, names);
            collect_stmts(body, names);
        }
        Stmt::If { clauses, else_body } => {
            for clause in clauses {
                collect_expr(&clause.cond, names);
                collect_stmts(&clause.body, names);
            }
            if let Some(body) = else_body {
                collect_stmts(body, names);
            }
        }
        Stmt::NumericFor { name, start, limit, step, body } => {
            names.insert(name.clone());
            collect_expr(start, names);
            collect_expr(limit, names);
            if let Some(step) = step {
                collect_expr(step, names);
            }
            collect_stmts(body, names);
        }
        Stmt::GenericFor { names: bound, iters, body } => {
            names.extend(bound.iter().cloned());
            collect_exprs(iters, names);
            collect_stmts(body, names);
        }
        Stmt::Return(values) => collect_exprs(values, names),
        Stmt::ExprStmt(expr) => collect_expr(expr, names),
        Stmt::ClassDecl(decl) => {
            names.insert(decl.name.clone());
            let members = decl
                .top_level_members
                .iter()
                .chain(decl.blocks.iter().flat_map(|block| block.members.iter()));
            for member in members {
                match member {
                    Member::Field(field) => {
                        if let Some(value) = &field.value {
                            collect_expr(value, names);
                        }
                    }
                    Member::Method(method) => {
                        collect_params(&method.params, names);
                        if let Some(body) = &method.body {
                            collect_stmts(body, names);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn collect_exprs(exprs: &[Expr], names: &mut HashSet<String>) {
    for expr in exprs {
        collect_expr(expr, names);
    }
}

fn collect_branch(branch: &IfExprBranch, names: &mut HashSet<String>) {
    collect_stmts(&branch.statements, names);
    collect_expr(&branch.result, names);
}

fn collect_expr(expr: &Expr, names: &mut HashSet<String>) {
    match expr {
        Expr::InterpolatedString(parts) => {
            for part in parts {
                if let InterpolatedPart::Expr(inner) = part {
                    collect_expr(inner, names);
                }
            }
        }
        Expr::Field { obj, .. } => collect_expr(obj, names),
        Expr::Index { obj, key } => {
            collect_expr(obj, names);
            collect_expr(key, names);
        }
        Expr::Call { callee, args } => {
            collect_expr(callee, names);
            collect_exprs(args, names);
        }
        Expr::MethodCall { obj, args, .. } => {
            collect_expr(obj, names);
            collect_exprs(args, names);
        }
        Expr::Unop { expr, .. } => collect_expr(expr, names),
        Expr::Binop { left, right, .. } => {
            collect_expr(left, names);
            collect_expr(right, names);
        }
        Expr::Table(fields) => {
            for field in fields {
                match field {
                    TableField::Index { key, value } => {
                        collect_expr(key, names);
                        collect_expr(value, names);
                    }
                    TableField::Name { value, .. } | TableField::Value(value) => {
                        collect_expr(value, names);
                    }
                }
            }
        }
        Expr::Function { params, body, .. } => {
            collect_params(params, names);
            collect_stmts(body, names);
        }
        Expr::If(if_expr) => {
            for clause in &if_expr.clauses {
                collect_expr(&clause.cond, names);
                collect_branch(&clause.branch, names);
            }
            collect_branch(&if_expr.else_branch, names);
        }
        Expr::Bind { name, value, .. } => {
            names.insert(name.clone());
            collect_expr(value, names);
        }
        _ => {}
    }
}

/// 変数・型名として参照される識別子の集合。テンプレート文字列内の語も含める。
/// 字句解析できないソースでは空集合を返す。
pub fn referenced_names(source: &str) -> HashSet<String> {
    let Ok(tokens) = Lexer::new(source).tokenize() else {
        return HashSet::new();
    };
    let roles = classify(&tokens);
    let mut names = HashSet::new();
    for (token, is_reference) in tokens.iter().zip(roles) {
        match &token.kind {
            TokenKind::Ident if is_reference => {
                names.insert(token.value.clone());
            }
            TokenKind::TemplateString => {
                names.extend(template_words(&token.value).map(|(_, word)| word.to_string()));
            }
            _ => {}
        }
    }
    names
}

/// `renames` に従って識別子の参照を書き換える。行数は変わらない。
pub fn rename_identifiers(
    source: &str,
    renames: &HashMap<String, String>,
) -> Result<String, String> {
    if renames.is_empty() {
        return Ok(source.to_string());
    }
    let tokens = Lexer::new(source)
        .tokenize()
        .map_err(|error| format!("cannot rename included names: {error}"))?;
    let roles = classify(&tokens);

    let mut edits: HashMap<usize, Vec<(usize, usize, &str)>> = HashMap::new();
    for (token, is_reference) in tokens.iter().zip(roles) {
        match &token.kind {
            TokenKind::Ident if is_reference => {
                if let Some(new_name) = renames.get(&token.value) {
                    edits
                        .entry(token.line)
                        .or_default()
                        .push((token.column, token.end_column, new_name));
                }
            }
            TokenKind::TemplateString => {
                for (_, word) in template_words(&token.value) {
                    if renames.contains_key(word) {
                        return Err(format!(
                            "'{word}' collides with a name in the including file but is used inside a template string, so it cannot be renamed automatically"
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    let mut lines: Vec<String> = source.lines().map(str::to_string).collect();
    for (line_number, mut line_edits) in edits {
        let Some(line) = lines.get_mut(line_number - 1) else {
            continue;
        };
        line_edits.sort_by(|a, b| b.0.cmp(&a.0));
        for (start, end, new_name) in line_edits {
            let (Some(start), Some(end)) = (byte_offset(line, start), byte_offset(line, end))
            else {
                continue;
            };
            line.replace_range(start..end, new_name);
        }
    }
    Ok(lines.join("\n"))
}

/// 一-based の UTF-16 桁を、行内のバイト位置へ変換する。
fn byte_offset(line: &str, utf16_column: usize) -> Option<usize> {
    let mut column = 1;
    for (offset, character) in line.char_indices() {
        if column == utf16_column {
            return Some(offset);
        }
        column += character.len_utf16();
    }
    (column == utf16_column).then_some(line.len())
}

/// テンプレート文字列中の、メンバーアクセスでない識別子らしい語。
fn template_words(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let bytes = text.as_bytes();
    let mut words = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_alphabetic() || byte == b'_' {
            let start = index;
            while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let after_dot = start > 0 && bytes[start - 1] == b'.';
            if !after_dot {
                words.push((start, &text[start..index]));
            }
        } else {
            index += 1;
        }
    }
    words.into_iter()
}

#[derive(Clone, Copy, PartialEq)]
enum Frame {
    Class,
    Members,
    Block,
    Repeat,
    Brace,
    Paren,
    Bracket,
}

/// 各Ident tokenが、変数・型名・クラス名の参照(または束縛)であれば `true`。
fn classify(tokens: &[Token]) -> Vec<bool> {
    let kind_at = |index: usize| tokens.get(index).map(|token| token.kind.clone());
    let mut stack: Vec<Frame> = Vec::new();
    let mut loop_header = false;
    let mut roles = vec![false; tokens.len()];

    for (index, token) in tokens.iter().enumerate() {
        let previous = index.checked_sub(1).and_then(kind_at);
        let next = kind_at(index + 1);
        match &token.kind {
            TokenKind::Class => stack.push(Frame::Class),
            TokenKind::Public | TokenKind::Private => stack.push(Frame::Members),
            TokenKind::Function | TokenKind::If => stack.push(Frame::Block),
            TokenKind::While | TokenKind::For => loop_header = true,
            TokenKind::Do => {
                if loop_header {
                    loop_header = false;
                } else {
                    stack.push(Frame::Block);
                }
            }
            TokenKind::Repeat => stack.push(Frame::Repeat),
            TokenKind::Until => {
                if stack.last() == Some(&Frame::Repeat) {
                    stack.pop();
                }
            }
            // 本体を持たない `function f() abstract` は `end` で閉じない。
            TokenKind::Abstract => {
                if stack.last() == Some(&Frame::Block) && previous != Some(TokenKind::Is) {
                    stack.pop();
                }
            }
            TokenKind::End => {
                stack.pop();
            }
            TokenKind::LBrace => stack.push(Frame::Brace),
            TokenKind::LParen => stack.push(Frame::Paren),
            TokenKind::LBracket => stack.push(Frame::Bracket),
            TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket => {
                stack.pop();
            }
            TokenKind::Ident => {
                roles[index] = is_reference(tokens, index, previous, next, &stack);
            }
            _ => {}
        }
    }
    roles
}

fn is_reference(
    tokens: &[Token],
    index: usize,
    previous: Option<TokenKind>,
    next: Option<TokenKind>,
    stack: &[Frame],
) -> bool {
    // `x.name`
    if previous == Some(TokenKind::Dot) {
        return false;
    }
    // `x:name(...)` のメソッド呼び出し。`a: Type` の型注釈とは `(` の有無で区別する。
    if previous == Some(TokenKind::Colon) && next == Some(TokenKind::LParen) {
        let receiver = index.checked_sub(2).map(|i| tokens[i].kind.clone());
        if matches!(
            receiver,
            Some(
                TokenKind::Ident
                    | TokenKind::RParen
                    | TokenKind::RBracket
                    | TokenKind::Self_
                    | TokenKind::Super
            )
        ) {
            return false;
        }
    }
    // `goto name` / `::name::`
    if previous == Some(TokenKind::Goto) {
        return false;
    }
    if previous == Some(TokenKind::DoubleColon) && next == Some(TokenKind::DoubleColon) {
        return false;
    }
    // `{ name = value }`
    if stack.last() == Some(&Frame::Brace)
        && matches!(
            previous,
            Some(TokenKind::LBrace | TokenKind::Comma | TokenKind::Semicolon)
        )
        && next == Some(TokenKind::Eq)
    {
        return false;
    }
    // クラス直下のメンバー宣言: `function name(` (Function frameは積み済み) と
    // `name = ...` / `name: Type`。クラス外の `function name(` は通常の参照。
    if previous == Some(TokenKind::Function) {
        let owner = stack.len().checked_sub(2).map(|i| stack[i]);
        if matches!(owner, Some(Frame::Class | Frame::Members)) {
            return false;
        }
    }
    if matches!(stack.last(), Some(Frame::Class | Frame::Members))
        && matches!(next, Some(TokenKind::Eq | TokenKind::Colon))
        && !matches!(previous, Some(TokenKind::Eq | TokenKind::Colon))
    {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rename(source: &str, pairs: &[(&str, &str)]) -> String {
        let renames = pairs
            .iter()
            .map(|(from, to)| (from.to_string(), to.to_string()))
            .collect();
        rename_identifiers(source, &renames).unwrap()
    }

    #[test]
    fn renames_references_but_not_members_or_keys() {
        let output = rename(
            "local module = {}\nmodule.module = 1\nlocal t = { module = module }\nobj:module()\nprint(module)",
            &[("module", "module__m")],
        );
        assert_eq!(
            output,
            "local module__m = {}\nmodule__m.module = 1\nlocal t = { module = module__m }\nobj:module()\nprint(module__m)"
        );
    }

    #[test]
    fn renames_class_names_types_and_inheritance_but_not_member_declarations() {
        let output = rename(
            "class Dog is\n    public is\n        Dog = 1\n        function Dog()\n        end\n        function eq(other: Dog)\n            return Dog.new()\n        end\n    end\nend\nclass Pup is Dog\nend",
            &[("Dog", "Dog__m")],
        );
        assert!(output.contains("class Dog__m is"));
        assert!(output.contains("        Dog = 1"), "{output}");
        assert!(output.contains("        function Dog()"), "{output}");
        assert!(output.contains("other: Dog__m"));
        assert!(output.contains("return Dog__m.new()"));
        assert!(output.contains("class Pup is Dog__m"));
    }

    #[test]
    fn abstract_methods_do_not_unbalance_blocks() {
        let output = rename(
            "class A is abstract\n    public is\n        function run() abstract\n    end\nend\nlocal x = A\nlocal f = function()\n    return x\nend",
            &[("x", "x__m")],
        );
        assert!(output.contains("local x__m = A"));
        assert!(output.contains("return x__m"));
    }

    #[test]
    fn leaves_strings_comments_and_labels_alone() {
        let output = rename(
            "local x = 'x' -- x\ngoto x\n::x::\nprint(x)",
            &[("x", "y")],
        );
        assert_eq!(output, "local y = 'x' -- x\ngoto x\n::x::\nprint(y)");
    }

    #[test]
    fn template_string_use_of_a_renamed_name_is_an_error() {
        let renames = HashMap::from([("name".to_string(), "name__m".to_string())]);
        assert!(rename_identifiers("local name = 1\nprint(`{name}`)", &renames).is_err());
        let ok = rename_identifiers("local name = 1\nprint(`{self.name}`)", &renames);
        assert!(ok.is_ok());
    }

    #[test]
    fn collects_top_level_names_and_references() {
        let names =
            top_level_names("local a = 1\nconst b = 2\nfunction f() end\nclass C is\nend\n")
                .unwrap();
        assert_eq!(names, vec!["a", "b", "f", "C"]);
        let used = referenced_names("local a = b.c\nprint(`{d}`)\n");
        assert!(used.contains("a") && used.contains("b") && used.contains("d"));
        assert!(!used.contains("c"));
    }

    #[test]
    fn collects_bound_names_at_any_depth() {
        let names = bound_names(
            "local a = 1\nfunction f(p) local q = 2 end\nfor i = 1, 2 do local r = function(s) end end\nif t := g() then end\nclass C is\n    public is\n        function m(u) end\n    end\nend\nprint(unbound)",
        )
        .unwrap();
        for name in ["a", "f", "p", "q", "i", "r", "s", "t", "C", "u"] {
            assert!(names.contains(name), "{name} should be bound");
        }
        assert!(!names.contains("unbound") && !names.contains("print"));
    }

    #[test]
    fn handles_non_ascii_columns() {
        let output = rename("local あ = 1 local x = x", &[("x", "y")]);
        assert_eq!(output, "local あ = 1 local y = y");
    }
}
