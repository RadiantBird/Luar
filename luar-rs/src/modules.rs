use crate::ast::{ClassDecl, Param, Stmt, TypeExpr};
use crate::lexer::{Lexer, Token, TokenKind};
use crate::parser::Parser;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// `.luard` の `declare class` が宣言するメソッド。本体は持たない。
#[derive(Debug, Clone)]
pub struct DeclaredMethod {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Option<TypeExpr>,
    pub is_static: bool,
}

/// `.luard` の `declare [global] function`。本体は持たない。
#[derive(Debug, Clone)]
pub struct DeclaredFunction {
    pub name: String,
    pub is_global: bool,
    pub type_params: Vec<String>,
    pub params: Vec<Param>,
    pub return_type: Option<TypeExpr>,
}

/// `.luard` の `[export] type`。`export` のないものは同じ `.luard` の中だけで使える。
#[derive(Debug, Clone)]
pub struct DeclaredType {
    pub name: String,
    pub is_export: bool,
    pub type_params: Vec<String>,
    pub ty: TypeExpr,
}

/// `.luard` の `declare class`。メンバーはすべてpublic。
#[derive(Debug, Clone)]
pub struct DeclaredClass {
    pub name: String,
    pub type_params: Vec<String>,
    pub parent: Option<String>,
    pub fields: Vec<(String, TypeExpr)>,
    pub methods: Vec<DeclaredMethod>,
    /// 通常のclass構文(`declare class X is ... end`)で書かれた宣言。
    /// アクセス修飾子とfriendを保つため、その場合はこちらを正とする。
    pub decl: Option<ClassDecl>,
}

#[derive(Debug, Clone, Default)]
pub struct ModuleDefinition {
    pub name: String,
    pub members: HashSet<String>,
    pub globals: HashSet<String>,
    /// `members` / `globals` の型。宣言順。
    pub member_types: Vec<(String, TypeExpr)>,
    pub global_types: Vec<(String, TypeExpr)>,
    pub classes: Vec<DeclaredClass>,
    pub functions: Vec<DeclaredFunction>,
    pub types: Vec<DeclaredType>,
}

#[derive(Debug, Clone)]
pub struct ModuleError {
    pub message: String,
    pub line: usize,
}

pub fn definition_path(module_name: &str, source_path: &Path) -> std::path::PathBuf {
    let directory = source_path.parent().unwrap_or_else(|| Path::new(""));
    directory.join(format!("{module_name}.luard"))
}

/// `import type name [from "path"]` が指す `.luard` のパス。
/// `path` がなければ、書いたファイルと同じディレクトリの `name.luard`。
/// あれば書いたファイルからの相対パス(`..` とサブディレクトリを含められる)で、`.luard` だけを受け付ける。
pub fn resolve_definition_path(
    module_name: &str,
    import_path: Option<&str>,
    source_path: &Path,
) -> Result<std::path::PathBuf, String> {
    let Some(requested) = import_path else {
        return Ok(definition_path(module_name, source_path));
    };
    if Path::new(requested).is_absolute() || requested.starts_with(['/', '\\']) {
        return Err("import type paths must be relative to the importing source file".to_string());
    }
    let extension = Path::new(requested)
        .extension()
        .and_then(|extension| extension.to_str());
    if extension != Some("luard") {
        return Err("import type only accepts .luard files".to_string());
    }
    let directory = source_path.parent().unwrap_or_else(|| Path::new(""));
    Ok(normalize_path(&directory.join(requested)))
}

/// `.` と `..` を字面で畳む(ファイルの有無は見ない)。
fn normalize_path(path: &Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut normalized = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let popped = matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                ) && normalized.pop();
                if !popped {
                    normalized.push("..");
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

pub fn load_definition(
    module_name: &str,
    source_path: &Path,
) -> Result<ModuleDefinition, ModuleError> {
    let definition_path = definition_path(module_name, source_path);
    let bytes = fs::read(&definition_path).map_err(|error| ModuleError {
        message: format!(
            "cannot read module definition '{}': {error}",
            definition_path.display()
        ),
        line: 0,
    })?;
    let source = String::from_utf8(bytes).map_err(|error| ModuleError {
        message: format!(
            "module definition '{}' is not valid UTF-8: {error}",
            definition_path.display()
        ),
        line: 0,
    })?;
    parse_definition(module_name, &definition_path, &source)
}

/// `load_definition` の、読めた分も返す版。ファイルが読めなければ定義は `None`。
pub fn load_definition_lossy(
    module_name: &str,
    source_path: &Path,
) -> (Option<ModuleDefinition>, Vec<ModuleError>) {
    load_definition_file_lossy(module_name, &definition_path(module_name, source_path))
}

/// 解決済みのパスの `.luard` を、読めた分まで読む。
pub fn load_definition_file_lossy(
    module_name: &str,
    definition_path: &Path,
) -> (Option<ModuleDefinition>, Vec<ModuleError>) {
    let source = match fs::read(&definition_path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(source) => source,
            Err(error) => {
                return (
                    None,
                    vec![ModuleError {
                        message: format!(
                            "module definition '{}' is not valid UTF-8: {error}",
                            definition_path.display()
                        ),
                        line: 0,
                    }],
                );
            }
        },
        Err(error) => {
            return (
                None,
                vec![ModuleError {
                    message: format!(
                        "cannot read module definition '{}': {error}",
                        definition_path.display()
                    ),
                    line: 0,
                }],
            );
        }
    };
    let (definition, errors) = parse_definition_lossy(module_name, definition_path, &source);
    (Some(definition), errors)
}

/// 最初のエラーで失敗する読み込み。
pub fn parse_definition(
    module_name: &str,
    definition_path: &Path,
    source: &str,
) -> Result<ModuleDefinition, ModuleError> {
    let (definition, mut errors) = parse_definition_lossy(module_name, definition_path, source);
    if errors.is_empty() {
        Ok(definition)
    } else {
        Err(errors.remove(0))
    }
}

/// `.luard` を読めるだけ読む。壊れた宣言は次の `declare` まで読み飛ばして、
/// それ以外の宣言は残す。エラーは宣言ごとに1つずつ返す。
pub fn parse_definition_lossy(
    module_name: &str,
    definition_path: &Path,
    source: &str,
) -> (ModuleDefinition, Vec<ModuleError>) {
    let mut builder = DefinitionBuilder::default();
    let mut errors = Vec::new();
    let mut lexer = Lexer::new(source);
    match lexer.tokenize() {
        Err(message) => errors.push(ModuleError {
            message: format!("{}: {message}", definition_path.display()),
            line: 0,
        }),
        Ok(tokens) => {
            let mut parser = DefinitionParser {
                tokens,
                pos: 0,
                path: definition_path,
            };
            while !parser.at(TokenKind::Eof) {
                let item_start = parser.pos;
                if let Err(error) = parser.parse_item(&mut builder) {
                    errors.push(error);
                    parser.pos = parser.pos.max(item_start + 1);
                    parser.resynchronize(item_start + 1);
                }
            }
        }
    }
    (builder.finish(module_name), errors)
}

/// 読み込み中の宣言の集まり。
#[derive(Default)]
struct DefinitionBuilder {
    members: HashSet<String>,
    globals: HashSet<String>,
    member_types: Vec<(String, TypeExpr)>,
    global_types: Vec<(String, TypeExpr)>,
    classes: Vec<DeclaredClass>,
    functions: Vec<DeclaredFunction>,
    types: Vec<DeclaredType>,
    all_names: HashSet<String>,
}

impl DefinitionBuilder {
    fn finish(self, module_name: &str) -> ModuleDefinition {
        ModuleDefinition {
            name: module_name.to_string(),
            members: self.members,
            globals: self.globals,
            member_types: self.member_types,
            global_types: self.global_types,
            classes: self.classes,
            functions: self.functions,
            types: self.types,
        }
    }
}

struct DefinitionParser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    path: &'a Path,
}

impl DefinitionParser<'_> {
    fn token(&self) -> &Token {
        &self.tokens[self.pos]
    }

    /// トップレベルの宣言を1つ読んで `builder` へ入れる。
    fn parse_item(&mut self, builder: &mut DefinitionBuilder) -> Result<(), ModuleError> {
        let type_params = self.parse_template_header()?;
        if let Some(declared) = self.parse_declared_type(&type_params)? {
            if builder.types.iter().any(|ty| ty.name == declared.name) {
                return Err(self.error(
                    self.previous_line(),
                    format!("type '{}' is declared more than once", declared.name),
                ));
            }
            builder.types.push(declared);
            return Ok(());
        }
        self.expect(TokenKind::Declare, "'declare'")?;
        if self.at(TokenKind::Function) || self.at_global_function() {
            let is_global = self.take(TokenKind::Global);
            let function = self.parse_declared_function(is_global, type_params)?;
            if !builder.all_names.insert(function.name.clone()) {
                return Err(self.error(
                    self.previous_line(),
                    format!("name '{}' is declared more than once", function.name),
                ));
            }
            if function.is_global {
                builder.globals.insert(function.name.clone());
            } else {
                builder.members.insert(function.name.clone());
            }
            builder.functions.push(function);
            return Ok(());
        }
        if self.at(TokenKind::Class) {
            let mut class = self.parse_declared_class()?;
            class.type_params = type_params;
            if !builder.all_names.insert(class.name.clone()) {
                return Err(self.error(
                    self.previous_line(),
                    format!("name '{}' is declared more than once", class.name),
                ));
            }
            builder.classes.push(class);
            return Ok(());
        }
        if !type_params.is_empty() {
            return Err(self.error(
                self.previous_line(),
                "'template' can only be applied to a type, 'declare function' or 'declare class'"
                    .to_string(),
            ));
        }
        let is_global = self.take(TokenKind::Global);
        if is_global && self.at(TokenKind::Class) {
            return Err(self.error(
                self.token().line,
                "a declared class is already global; use `declare class Name is ... end` without 'global'"
                    .to_string(),
            ));
        }
        let (name, line) = self.expect_ident()?;
        self.expect(TokenKind::Colon, "':'")?;
        let ty = self.parse_definition_type()?;
        if !builder.all_names.insert(name.clone()) {
            return Err(self.error(line, format!("name '{name}' is declared more than once")));
        }
        if is_global {
            builder.globals.insert(name.clone());
            builder.global_types.push((name, ty));
        } else {
            builder.members.insert(name.clone());
            builder.member_types.push((name, ty));
        }
        Ok(())
    }

    /// エラーの後、次の `declare` まで読み飛ばす。`declare` は宣言の外にしか現れないので、
    /// 壊れたクラスの残りを丸ごと飛ばせる。直前に `template <...>` があれば、`floor` 以降に限ってそこから読み直す。
    fn resynchronize(&mut self, floor: usize) {
        while !self.at(TokenKind::Eof) && !self.at(TokenKind::Declare) {
            self.pos += 1;
        }
        if !self.at(TokenKind::Declare) || self.pos == 0 {
            return;
        }
        // `template <A, B> declare ...` の `template` まで戻る。
        let mut back = self.pos;
        if self.tokens[back - 1].kind == TokenKind::Gt {
            while back > 0 && self.tokens[back - 1].kind != TokenKind::Lt {
                back -= 1;
            }
            let template_at = back.checked_sub(2);
            if let Some(index) = template_at {
                if index >= floor
                    && self.tokens[index].kind == TokenKind::Ident
                    && self.tokens[index].value == "template"
                {
                    self.pos = index;
                }
            }
        }
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.token().kind == kind
    }

    fn take(&mut self, kind: TokenKind) -> bool {
        if self.at(kind) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: TokenKind, expected: &str) -> Result<(), ModuleError> {
        if self.take(kind) {
            Ok(())
        } else {
            Err(self.error(
                self.token().line,
                format!("expected {expected}, got '{}'", self.token().value),
            ))
        }
    }

    fn expect_ident(&mut self) -> Result<(String, usize), ModuleError> {
        if self.at(TokenKind::Ident) {
            let token = self.token().clone();
            self.pos += 1;
            Ok((token.value, token.line))
        } else {
            Err(self.error(
                self.token().line,
                format!("expected identifier, got '{}'", self.token().value),
            ))
        }
    }

    fn previous_line(&self) -> usize {
        self.pos
            .checked_sub(1)
            .map(|index| self.tokens[index].line)
            .unwrap_or(0)
    }

    /// `declare class Name [is Parent] ... end` を読む。呼び出し時点で `class` の前。
    /// メンバーは `name: Type` のフィールドと、本体のない
    /// `[static] function name(params)[: Return]` で、`end` で閉じる。
    fn parse_declared_class(&mut self) -> Result<DeclaredClass, ModuleError> {
        // `declare class Name is` は通常のclass構文(本体・public/private・friend)として読む。
        // 本体は構文だけ検査し、コードは生成しない。`is` のない短い形式は下の宣言専用の文法。
        let start = self.pos;
        if self.tokens.get(start + 2).map(|token| &token.kind) == Some(&TokenKind::Is) {
            // 本体なし(シグネチャだけ)の書き方を先に試す。クラスの直後が次の `declare` か
            // 末尾で終われば採用し、そうでなければ本体つきの通常のclass構文として読む。
            let tokens = self.tokens[start..].to_vec();
            let mut bodyless = Parser::from_tokens(tokens.clone()).with_bodyless_methods();
            let bodyless_result = bodyless.parse_class();
            // 本体つきでも読めなかったとき、より先まで進んだ側のエラーを報告する。
            let bodyless_error = bodyless_result
                .as_ref()
                .err()
                .map(|error| (error.span.line, error.message.clone(), error.definitive));
            let bodyless_decl = bodyless_result.ok().filter(|_| {
                matches!(bodyless.next_kind(), TokenKind::Declare | TokenKind::Eof)
            });
            let (decl, consumed) = match bodyless_decl {
                Some(decl) => (decl, bodyless.position()),
                None => {
                    let mut parser = Parser::from_tokens(tokens);
                    let decl = parser.parse_class().map_err(|error| {
                        match bodyless_error {
                            Some((line, message, definitive))
                                if definitive || line >= error.span.line =>
                            {
                                self.error(line, message)
                            }
                            _ => self.error(error.span.line, error.message.clone()),
                        }
                    })?;
                    (decl, parser.position())
                }
            };
            self.pos = start + consumed;
            return Ok(DeclaredClass {
                name: decl.name.clone(),
                parent: decl.parent.clone(),
                type_params: Vec::new(),
                fields: Vec::new(),
                methods: Vec::new(),
                decl: Some(decl),
            });
        }
        self.expect(TokenKind::Class, "'class'")?;
        let (name, _) = self.expect_ident()?;
        let parent = if self.take(TokenKind::Is) {
            Some(self.expect_ident()?.0)
        } else {
            None
        };
        let mut fields: Vec<(String, TypeExpr)> = Vec::new();
        let mut methods: Vec<DeclaredMethod> = Vec::new();
        while !self.take(TokenKind::End) {
            if self.at(TokenKind::Eof) {
                return Err(self.error(
                    self.token().line,
                    format!("expected 'end' to close 'declare class {name}'"),
                ));
            }
            let is_static = self.take(TokenKind::Static);
            if self.take(TokenKind::Function) {
                let (method_name, line) = self.expect_ident()?;
                self.expect(TokenKind::LParen, "'('")?;
                let params = self.parse_declared_params()?;
                self.expect(TokenKind::RParen, "')'")?;
                let return_type = if self.take(TokenKind::Colon) {
                    Some(self.parse_definition_type()?)
                } else {
                    None
                };
                if methods.iter().any(|method| method.name == method_name)
                    || fields.iter().any(|(field, _)| *field == method_name)
                {
                    return Err(self.error(
                        line,
                        format!("member '{method_name}' is declared more than once in '{name}'"),
                    ));
                }
                methods.push(DeclaredMethod {
                    name: method_name,
                    params,
                    return_type,
                    is_static,
                });
                continue;
            }
            if is_static {
                return Err(self.error(
                    self.token().line,
                    "'static' can only be used before 'function'".to_string(),
                ));
            }
            let (field_name, line) = self.expect_ident()?;
            self.expect(TokenKind::Colon, "':'")?;
            let ty = self.parse_definition_type()?;
            if methods.iter().any(|method| method.name == field_name)
                || fields.iter().any(|(field, _)| *field == field_name)
            {
                return Err(self.error(
                    line,
                    format!("member '{field_name}' is declared more than once in '{name}'"),
                ));
            }
            fields.push((field_name, ty));
        }
        Ok(DeclaredClass {
            name,
            type_params: Vec::new(),
            parent,
            fields,
            methods,
            decl: None,
        })
    }

    fn parse_declared_params(&mut self) -> Result<Vec<Param>, ModuleError> {
        let mut params = Vec::new();
        // メソッドの `self` は自動で渡されるので、引数には数えない。
        if self.take(TokenKind::Self_) && !self.take(TokenKind::Comma) {
            return Ok(params);
        }
        if self.at(TokenKind::RParen) {
            return Ok(params);
        }
        loop {
            if self.take(TokenKind::DotDotDot) {
                params.push(Param::Vararg);
            } else {
                let (name, _) = self.expect_ident()?;
                let ty = if self.take(TokenKind::Colon) {
                    Some(self.parse_definition_type()?)
                } else {
                    None
                };
                params.push(Param::Named { name, ty });
            }
            if !self.take(TokenKind::Comma) {
                return Ok(params);
            }
        }
    }

    /// `.luard` の型式。`.luar` と同じ文法を共有パーサーで読む。
    fn parse_definition_type(&mut self) -> Result<TypeExpr, ModuleError> {
        let mut parser = Parser::from_tokens(self.tokens[self.pos..].to_vec());
        let ty = parser
            .parse_type()
            .map_err(|error| self.error(error.span.line, error.message))?;
        self.pos += parser.position();
        Ok(ty)
    }

    /// `template <T, U>`。なければ空。
    fn parse_template_header(&mut self) -> Result<Vec<String>, ModuleError> {
        let mut parser = Parser::from_tokens(self.tokens[self.pos..].to_vec());
        let type_params = parser
            .parse_optional_template_header()
            .map_err(|error| self.error(error.span.line, error.message))?;
        self.pos += parser.position();
        Ok(type_params)
    }

    /// 現在位置が `[export] type Name = ...` ならそれを読む。
    fn parse_declared_type(
        &mut self,
        type_params: &[String],
    ) -> Result<Option<DeclaredType>, ModuleError> {
        let mut parser = Parser::from_tokens(self.tokens[self.pos..].to_vec());
        if !parser.starts_type_alias() {
            return Ok(None);
        }
        let stmt = parser
            .parse_type_alias_with(type_params.to_vec())
            .map_err(|error| self.error(error.span.line, error.message))?;
        self.pos += parser.position();
        let Stmt::TypeAlias {
            is_export,
            name,
            type_params,
            ty,
            ..
        } = stmt
        else {
            unreachable!("parse_type_alias_with always returns a type alias");
        };
        Ok(Some(DeclaredType {
            name,
            is_export,
            type_params,
            ty,
        }))
    }

    /// `declare global function` の `global` を読む前か。
    fn at_global_function(&self) -> bool {
        self.at(TokenKind::Global)
            && self.tokens.get(self.pos + 1).map(|token| &token.kind) == Some(&TokenKind::Function)
    }

    /// `function name(params)[: Return]`。呼び出し時点で `function` の前。
    fn parse_declared_function(
        &mut self,
        is_global: bool,
        type_params: Vec<String>,
    ) -> Result<DeclaredFunction, ModuleError> {
        self.expect(TokenKind::Function, "'function'")?;
        let (name, _) = self.expect_ident()?;
        self.expect(TokenKind::LParen, "'('")?;
        let params = self.parse_declared_params()?;
        self.expect(TokenKind::RParen, "')'")?;
        let return_type = if self.take(TokenKind::Colon) {
            Some(self.parse_definition_type()?)
        } else {
            None
        };
        Ok(DeclaredFunction {
            name,
            is_global,
            type_params,
            params,
            return_type,
        })
    }

    fn error(&self, line: usize, message: String) -> ModuleError {
        ModuleError {
            message: format!("{}:{line}: {message}", self.path.display()),
            line,
        }
    }
}
