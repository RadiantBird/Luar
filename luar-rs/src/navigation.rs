//! エディタ向けの、意味による色分け(semantic tokens)と定義ジャンプ。
//!
//! `!include` 展開後のソースを `symbols` で解析し、行番号を `ExpandedSource` の対応表で
//! 元のファイルへ戻す。メンバー参照(`x.name`)の解決だけは型が必要なので、補完と同じ
//! チェッカーの問い合わせを使う。

use crate::CompileOptions;
use crate::include::{self, ExpandedSource, IncludeLine, SourceOrigin};
use crate::lexer::{Lexer, Token, TokenKind};
use crate::modules;
use crate::symbols::{self, SymbolKind, TokenAnalysis};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SemanticToken {
    /// 0始まり。
    pub line: usize,
    pub column: usize,
    pub length: usize,
    #[serde(rename = "type")]
    pub kind: String,
    pub modifiers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub file: String,
    /// 0始まり。
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenReport {
    pub tokens: Vec<SemanticToken>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DefinitionReport {
    pub locations: Vec<Location>,
}

/// 展開後のトークン列と、元ファイルへの対応。
struct View {
    analysis: TokenAnalysis,
    origins: Vec<SourceOrigin>,
    main_file: String,
    includes: Vec<IncludeLine>,
    include_line_numbers: HashSet<usize>,
}

fn lex(source: &str) -> Option<Vec<Token>> {
    Lexer::new(source).tokenize().ok()
}

fn is_definition_file(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()) == Some("luard")
}

/// `\\?\C:\...` のような拡張パス表記を通常のパスへ戻す。
fn display_path(path: &str) -> String {
    path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
}

fn build_view(source: &str, options: &CompileOptions) -> Option<View> {
    let path = options.source_path.as_deref()?;
    let main_file = path.display().to_string();
    let includes = include::include_lines(source);
    let include_line_numbers = includes.iter().map(|line| line.line).collect();

    if is_definition_file(path) {
        let analysis = symbols::analyze_definition(lex(source)?);
        let origins = source
            .lines()
            .enumerate()
            .map(|(index, _)| SourceOrigin {
                file: main_file.clone(),
                line: index + 1,
            })
            .collect();
        return Some(View {
            analysis,
            origins,
            main_file,
            includes,
            include_line_numbers,
        });
    }

    let expanded = include::expand_source(source, Some(path), options.target)
        .ok()
        .unwrap_or_else(|| blanked_includes(source, &main_file, &includes));
    let analysis = symbols::analyze_with(
        lex(&expanded.source)?,
        crate::stdlib::Builtins::for_target(options.target),
    );
    Some(View {
        analysis,
        origins: expanded.origins,
        main_file,
        includes,
        include_line_numbers,
    })
}

/// include展開に失敗したとき用に、include宣言の行を空行にして行番号を保つ。
fn blanked_includes(source: &str, main_file: &str, includes: &[IncludeLine]) -> ExpandedSource {
    let lines: Vec<String> = source
        .lines()
        .enumerate()
        .map(|(index, line)| {
            if includes.iter().any(|include| include.line == index + 1) {
                String::new()
            } else {
                line.to_string()
            }
        })
        .collect();
    ExpandedSource {
        origins: (0..lines.len())
            .map(|index| SourceOrigin {
                file: main_file.to_string(),
                line: index + 1,
            })
            .collect(),
        source: lines.join("\n"),
    }
}

impl View {
    fn origin(&self, token: &Token) -> Option<&SourceOrigin> {
        self.origins.get(token.line.checked_sub(1)?)
    }

    fn location_of(&self, index: usize) -> Option<Location> {
        let token = &self.analysis.tokens[index];
        let origin = self.origin(token)?;
        Some(Location {
            file: display_path(&origin.file),
            line: origin.line.checked_sub(1)?,
            column: token.column.checked_sub(1)?,
            end_line: origin.line.checked_sub(1)?,
            end_column: token.end_column.checked_sub(1)?,
        })
    }

    /// 元ファイルの(0始まりの行, 列)にあるIdent token。単語の右端にカーソルがある場合も、
    /// 他のtokenと重ならなければその単語とみなす。
    fn token_at(&self, line: usize, column: usize) -> Option<usize> {
        let find = |inclusive_end: bool| {
            self.analysis.tokens.iter().position(|token| {
                let end = token.end_column - 1 + usize::from(inclusive_end);
                token.kind == TokenKind::Ident
                    && token.line == token.end_line
                    && token.column - 1 <= column
                    && column < end
                    && self.origin(token).is_some_and(|origin| {
                        origin.file == self.main_file
                            && origin.line == line + 1
                            && !self.include_line_numbers.contains(&origin.line)
                    })
            })
        };
        find(false).or_else(|| find(true))
    }

    /// 宣言が `local name = !include(...)` の行(展開後は別名の行)にあるか。
    fn declared_on_include_line(&self, declaration: usize) -> bool {
        self.origin(&self.analysis.tokens[declaration])
            .is_some_and(|origin| {
                origin.file == self.main_file
                    && self.include_line_numbers.contains(&origin.line)
            })
    }
}

fn kind_modifiers(entry: &symbols::Entry) -> Vec<String> {
    let mut modifiers = Vec::new();
    if entry.declaration {
        modifiers.push("declaration".to_string());
    }
    if entry.readonly {
        modifiers.push("readonly".to_string());
    }
    if entry.is_static {
        modifiers.push("static".to_string());
    }
    if entry.builtin {
        modifiers.push("defaultLibrary".to_string());
    }
    modifiers
}

/// 変数・const・関数・メソッド・フィールド・クラス・引数・モジュールの色分け。
pub fn semantic_tokens(source: &str, options: &CompileOptions) -> Vec<SemanticToken> {
    let Some(view) = build_view(source, options) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let definitions = options
        .source_path
        .as_deref()
        .map(|path| definition_files(&view, path))
        .unwrap_or_default();
    for (token, entry) in view.analysis.tokens.iter().zip(&view.analysis.entries) {
        let Some(mut kind) = entry.kind else { continue };
        // `import type` した `.luard` が宣言するクラス名は、クラスとして色付けする。
        if entry.unresolved
            && definitions
                .iter()
                .any(|file| file.analysis.class_decls.contains_key(&token.value))
        {
            kind = SymbolKind::Class;
        }
        let Some(origin) = view.origin(token) else {
            continue;
        };
        // `!include` で束縛した名前は、モジュールとして色付けする。
        if entry.decl.is_some_and(|declaration| view.declared_on_include_line(declaration)) {
            kind = SymbolKind::Namespace;
        }
        // include先の本文と、include宣言行から作った別名の行は、このファイルの表示ではない。
        if origin.file != view.main_file || view.include_line_numbers.contains(&origin.line) {
            continue;
        }
        result.push(SemanticToken {
            line: origin.line - 1,
            column: token.column - 1,
            length: token.value.encode_utf16().count(),
            kind: kind.name().to_string(),
            modifiers: kind_modifiers(entry),
        });
    }
    for include in &view.includes {
        result.push(SemanticToken {
            line: include.line - 1,
            column: include.name_column,
            length: include.name.encode_utf16().count(),
            kind: SymbolKind::Namespace.name().to_string(),
            modifiers: vec!["declaration".to_string()],
        });
    }
    result.sort_by_key(|token| (token.line, token.column));
    result.dedup_by_key(|token| (token.line, token.column));
    result
}

/// UTF-16オフセットを(0始まりの行, 列)へ。
fn position_of(source: &str, utf16_offset: usize) -> (usize, usize) {
    let (mut line, mut column, mut units) = (0, 0, 0);
    for character in source.chars() {
        if units >= utf16_offset {
            break;
        }
        units += character.len_utf16();
        if character == '\n' {
            line += 1;
            column = 0;
        } else {
            column += character.len_utf16();
        }
    }
    (line, column)
}

/// (0始まりの行, 列)をUTF-16オフセットへ。
fn offset_of(source: &str, line: usize, column: usize) -> usize {
    let mut offset = 0;
    for (index, text) in source.split_inclusive('\n').enumerate() {
        if index == line {
            let mut units = 0;
            for character in text.chars() {
                if units >= column {
                    break;
                }
                units += character.len_utf16();
            }
            return offset + units;
        }
        offset += text.encode_utf16().count();
    }
    offset
}

/// `import type` されたモジュールの `.luard`。
struct DefinitionFile {
    module: String,
    path: PathBuf,
    analysis: TokenAnalysis,
}

impl DefinitionFile {
    fn location(&self, index: usize) -> Location {
        let token = &self.analysis.tokens[index];
        Location {
            file: self.path.display().to_string(),
            line: token.line - 1,
            column: token.column - 1,
            end_line: token.end_line - 1,
            end_column: token.end_column - 1,
        }
    }

    fn top_of_file(&self) -> Location {
        Location {
            file: self.path.display().to_string(),
            line: 0,
            column: 0,
            end_line: 0,
            end_column: 0,
        }
    }
}

fn imported_modules(view: &View) -> Vec<String> {
    let tokens = &view.analysis.tokens;
    let mut names = Vec::new();
    for index in 0..tokens.len().saturating_sub(2) {
        if tokens[index].kind == TokenKind::Import
            && tokens[index + 1].kind == TokenKind::Ident
            && tokens[index + 1].value == "type"
            && tokens[index + 2].kind == TokenKind::Ident
        {
            names.push(tokens[index + 2].value.clone());
        }
    }
    names
}

fn definition_files(view: &View, source_path: &Path) -> Vec<DefinitionFile> {
    imported_modules(view)
        .into_iter()
        .filter_map(|module| {
            let path = modules::definition_path(&module, source_path);
            let text = std::fs::read_to_string(&path).ok()?;
            let analysis = symbols::analyze_definition(lex(&text)?);
            Some(DefinitionFile {
                module,
                path,
                analysis,
            })
        })
        .collect()
}

fn resolve_include_path(source_path: &Path, include: &IncludeLine) -> Option<PathBuf> {
    let requested = include.path.strip_prefix('@').unwrap_or(&include.path);
    let directory = source_path.parent().unwrap_or_else(|| Path::new(""));
    let path = std::fs::canonicalize(directory.join(requested)).ok()?;
    Some(PathBuf::from(display_path(&path.display().to_string())))
}

fn file_start(path: &Path) -> Location {
    Location {
        file: path.display().to_string(),
        line: 0,
        column: 0,
        end_line: 0,
        end_column: 0,
    }
}

/// カーソル位置の定義。複数見つかった場合は全て返す。
pub fn definition(source: &str, utf16_offset: usize, options: &CompileOptions) -> Vec<Location> {
    let Some(source_path) = options.source_path.as_deref() else {
        return Vec::new();
    };
    let (line, column) = position_of(source, utf16_offset);

    // `!include("./x.luar")` のパス文字列は、そのファイルを開く。
    for include in include::include_lines(source) {
        if include.line == line + 1
            && (include.path_start_column..include.path_end_column).contains(&column)
        {
            return resolve_include_path(source_path, &include)
                .map(|path| vec![file_start(&path)])
                .unwrap_or_default();
        }
    }

    let Some(view) = build_view(source, options) else {
        return Vec::new();
    };
    let Some(index) = view.token_at(line, column) else {
        // include宣言行の束縛名は、include先のファイルを開く。
        return view
            .includes
            .iter()
            .find(|include| {
                include.line == line + 1
                    && (include.name_column..include.name_column + include.name.len())
                        .contains(&column)
            })
            .and_then(|include| resolve_include_path(source_path, include))
            .map(|path| vec![file_start(&path)])
            .unwrap_or_default();
    };
    let entry = view.analysis.entries[index];
    let definitions = definition_files(&view, source_path);

    // `import type Name` の名前は、その `.luard` を開く。
    if entry.kind == Some(SymbolKind::Namespace) && entry.declaration {
        let name = &view.analysis.tokens[index].value;
        if let Some(file) = definitions.iter().find(|file| &file.module == name) {
            return vec![file.top_of_file()];
        }
    }
    if entry.is_member {
        return member_definition(source, &view, index, &entry, &definitions, options);
    }
    if let Some(declaration) = entry.decl {
        return view.location_of(declaration).into_iter().collect();
    }
    if entry.unresolved {
        return unresolved_definition(&view.analysis.tokens[index].value, &definitions);
    }
    Vec::new()
}

/// どのスコープにも宣言がない名前を、`import type` した `.luard` の宣言から探す。
fn unresolved_definition(name: &str, definitions: &[DefinitionFile]) -> Vec<Location> {
    let mut found = Vec::new();
    for file in definitions {
        if file.module == name {
            found.push(file.top_of_file());
        }
        if let Some(&index) = file.analysis.top_decls.get(name) {
            found.push(file.location(index));
        }
        if let Some(&index) = file.analysis.class_decls.get(name) {
            found.push(file.location(index));
        }
    }
    found
}

fn member_definition(
    source: &str,
    view: &View,
    index: usize,
    entry: &symbols::Entry,
    definitions: &[DefinitionFile],
    options: &CompileOptions,
) -> Vec<Location> {
    let token = &view.analysis.tokens[index];
    let name = token.value.as_str();
    let Some(origin) = view.origin(token) else {
        return Vec::new();
    };
    let probe_offset = offset_of(source, origin.line - 1, token.column - 1);
    let receiver = crate::probe_receiver(source, probe_offset, options);

    if let Some((info, receiver_text)) = &receiver {
        // クラスのインスタンス/クラス自身: 子から親へたどって最初に見つかった宣言。
        for class in &info.class_chain {
            let members = view
                .analysis
                .class_members
                .get(class)
                .and_then(|members| members.iter().find(|(member, _)| member == name));
            if let Some(&(_, declaration)) = members {
                if let Some(location) = view.location_of(declaration) {
                    return vec![location];
                }
            }
            for file in definitions {
                let members = file
                    .analysis
                    .class_members
                    .get(class)
                    .and_then(|members| members.iter().find(|(member, _)| member == name));
                if let Some(&(_, declaration)) = members {
                    return vec![file.location(declaration)];
                }
            }
        }
        // `import type` したモジュールのメンバー: `mod.name`。
        if let Some(file) = definitions.iter().find(|file| &file.module == receiver_text) {
            if let Some(&declaration) = file.analysis.top_decls.get(name) {
                return vec![file.location(declaration)];
            }
        }
        if info.is_shape {
            return field_definitions(view, index, name);
        }
        if !info.class_chain.is_empty() {
            return Vec::new();
        }
    }

    // 型が決まらないときは、同名のメンバー宣言を全て候補にする。
    let mut found = field_definitions(view, index, name);
    for members in view.analysis.class_members.values() {
        for &(ref member, declaration) in members {
            if member == name && declaration != index && !entry.declaration {
                found.extend(view.location_of(declaration));
            }
        }
    }
    for file in definitions {
        for members in file.analysis.class_members.values() {
            for &(ref member, declaration) in members {
                if member == name {
                    found.push(file.location(declaration));
                }
            }
        }
    }
    found.dedup();
    found
}

/// テーブルのキー・`m.x = ...`・`function m.run()` として定義されたフィールド。
fn field_definitions(view: &View, index: usize, name: &str) -> Vec<Location> {
    let mut found: Vec<Location> = view
        .analysis
        .field_keys
        .iter()
        .filter(|(field, declaration)| field == name && *declaration != index)
        .filter_map(|&(_, declaration)| view.location_of(declaration))
        .collect();
    found.dedup();
    found
}
