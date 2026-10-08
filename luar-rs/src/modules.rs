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

pub fn parse_definition(
    module_name: &str,
    definition_path: &Path,
    source: &str,
) -> Result<ModuleDefinition, ModuleError> {
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().map_err(|message| ModuleError {
        message: format!("{}: {message}", definition_path.display()),
        line: 0,
    })?;
    let mut parser = DefinitionParser {
        tokens,
        pos: 0,
        path: definition_path,
    };
    let mut members = HashSet::new();
    let mut globals = HashSet::new();
    let mut member_types = Vec::new();
    let mut global_types = Vec::new();
    let mut classes = Vec::new();
    let mut functions = Vec::new();
    let mut types: Vec<DeclaredType> = Vec::new();
    let mut all_names = HashSet::new();

    while !parser.at(TokenKind::Eof) {
        let type_params = parser.parse_template_header()?;
        if let Some(declared) = parser.parse_declared_type(&type_params)? {
            if types.iter().any(|ty| ty.name == declared.name) {
                return Err(parser.error(
                    parser.previous_line(),
                    format!("type '{}' is declared more than once", declared.name),
                ));
            }
            types.push(declared);
            continue;
        }
        parser.expect(TokenKind::Declare, "'declare'")?;
        if parser.at(TokenKind::Function) || parser.at_global_function() {
            let is_global = parser.take(TokenKind::Global);
            let function = parser.parse_declared_function(is_global, type_params)?;
            if !all_names.insert(function.name.clone()) {
                return Err(parser.error(
                    parser.previous_line(),
                    format!("name '{}' is declared more than once", function.name),
                ));
            }
            if function.is_global {
                globals.insert(function.name.clone());
            } else {
                members.insert(function.name.clone());
            }
            functions.push(function);
            continue;
        }
        if parser.at(TokenKind::Class) {
            let mut class = parser.parse_declared_class()?;
            class.type_params = type_params;
            if !all_names.insert(class.name.clone()) {
                return Err(parser.error(
                    parser.previous_line(),
                    format!("name '{}' is declared more than once", class.name),
                ));
            }
            classes.push(class);
            continue;
        }
        if !type_params.is_empty() {
            return Err(parser.error(
                parser.previous_line(),
                "'template' can only be applied to a type, 'declare function' or 'declare class'"
                    .to_string(),
            ));
        }
        let is_global = parser.take(TokenKind::Global);
        let (name, line) = parser.expect_ident()?;
        parser.expect(TokenKind::Colon, "':'")?;
        let ty = parser.parse_definition_type()?;
        if !all_names.insert(name.clone()) {
            return Err(parser.error(line, format!("name '{name}' is declared more than once")));
        }
        if is_global {
            globals.insert(name.clone());
            global_types.push((name, ty));
        } else {
            members.insert(name.clone());
            member_types.push((name, ty));
        }
    }

    Ok(ModuleDefinition {
        name: module_name.to_string(),
        members,
        globals,
        member_types,
        global_types,
        classes,
        functions,
        types,
    })
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
            let bodyless_decl = bodyless.parse_class().ok().filter(|_| {
                matches!(bodyless.next_kind(), TokenKind::Declare | TokenKind::Eof)
            });
            let (decl, consumed) = match bodyless_decl {
                Some(decl) => (decl, bodyless.position()),
                None => {
                    let mut parser = Parser::from_tokens(tokens);
                    let decl = parser
                        .parse_class()
                        .map_err(|error| self.error(error.span.line, error.message.clone()))?;
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
