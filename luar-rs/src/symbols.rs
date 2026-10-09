//! 字句ベースのスコープ解析。semantic tokens(意味による色分け)と定義ジャンプの土台。
//!
//! 構文解析(AST)はトークンの位置を持たないので、トークン列を直接たどって
//! 宣言と参照を結び付ける。入力途中で構文エラーになっているソースでも、字句解析が
//! できれば動く。型の判断が必要なメンバー参照の解決は、ここでは行わない(チェッカーの仕事)。

use crate::lexer::{Lexer, Token, TokenKind};
use crate::stdlib::{BuiltinKind, Builtins};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SymbolKind {
    Namespace,
    Class,
    Function,
    Method,
    Variable,
    Parameter,
    Property,
    Type,
    Keyword,
}

impl SymbolKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Namespace => "namespace",
            Self::Class => "class",
            Self::Function => "function",
            Self::Method => "method",
            Self::Variable => "variable",
            Self::Parameter => "parameter",
            Self::Property => "property",
            Self::Type => "type",
            Self::Keyword => "keyword",
        }
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Entry {
    pub kind: Option<SymbolKind>,
    pub declaration: bool,
    pub readonly: bool,
    pub is_static: bool,
    /// 宣言位置のtoken番号。宣言そのものでは自分自身。
    pub decl: Option<usize>,
    /// `x.name` / `x:name()` のメンバー。宣言はレシーバの型に依存する。
    pub is_member: bool,
    /// どのスコープにも宣言がない名前。
    pub unresolved: bool,
    /// 標準ライブラリの名前 (`print`、`math`、`math.floor`)。
    pub builtin: bool,
}

#[derive(Clone, Copy)]
struct Decl {
    kind: SymbolKind,
    token: usize,
    readonly: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameKind {
    Global,
    Class,
    Members,
    Function,
    Block,
    Repeat,
    Brace,
    Paren,
    Bracket,
}

struct Frame {
    kind: FrameKind,
    names: HashMap<String, Decl>,
    class: Option<String>,
}

impl Frame {
    fn new(kind: FrameKind) -> Self {
        Frame {
            kind,
            names: HashMap::new(),
            class: None,
        }
    }

    /// 名前の宣言を受け付けるスコープか。
    fn holds_names(&self) -> bool {
        matches!(
            self.kind,
            FrameKind::Global | FrameKind::Function | FrameKind::Block | FrameKind::Repeat
        )
    }
}

#[derive(Debug, Default)]
pub struct TokenAnalysis {
    pub tokens: Vec<Token>,
    pub entries: Vec<Entry>,
    /// クラス名 → (メンバー名, 宣言token)。`self.x = ...` による代入も含む。
    pub class_members: HashMap<String, Vec<(String, usize)>>,
    pub class_decls: HashMap<String, usize>,
    pub class_parents: HashMap<String, String>,
    /// テーブルのキー、`m.x = ...`、`function m.run()` で定義されるフィールド。
    pub field_keys: Vec<(String, usize)>,
    /// `.luard` の `declare [global] name: Type`。
    pub top_decls: HashMap<String, usize>,
    /// ブロックの開閉が釣り合っていたか。
    pub balanced: bool,
}

struct Analyzer {
    tokens: Vec<Token>,
    entries: Vec<Entry>,
    frames: Vec<Frame>,
    loop_header: bool,
    underflow: bool,
    /// メソッドを本体なし(`end` なし)として読む。`.luard` の宣言用。
    bodyless_methods: bool,
    class_members: HashMap<String, Vec<(String, usize)>>,
    class_decls: HashMap<String, usize>,
    class_parents: HashMap<String, String>,
    field_keys: Vec<(String, usize)>,
    top_decls: HashMap<String, usize>,
    /// 直後の宣言だけで見える `template <T>` の型引数 (名前, 宣言token)。
    template_params: Vec<(String, usize)>,
    /// `template` ヘッダを読んだ時点のスコープの深さ。そこへ戻ったら宣言が終わった。
    template_depth: usize,
    /// 標準ライブラリの名前。宣言のない名前がこれに当たれば標準の名前として扱う。
    builtins: Builtins,
}

const PRIMITIVE_TYPES: &[&str] = &[
    "number", "string", "boolean", "nil", "table", "function", "any", "unknown",
];

/// 通常のソース。メソッドは本体を持つ(abstractを除く)。
pub fn analyze(tokens: Vec<Token>) -> TokenAnalysis {
    run(tokens, false, Builtins::default())
}

/// 標準ライブラリの名前を認識する通常のソース。
pub fn analyze_with(tokens: Vec<Token>, builtins: Builtins) -> TokenAnalysis {
    run(tokens, false, builtins)
}

/// `.luard`。メソッドが本体なしの書き方を先に試し、ブロックが釣り合わなければ
/// 本体つきとして読み直す。
pub fn analyze_definition(tokens: Vec<Token>) -> TokenAnalysis {
    let bodyless = run(tokens.clone(), true, Builtins::default());
    if bodyless.balanced {
        return bodyless;
    }
    run(tokens, false, Builtins::default())
}

/// テンプレート文字列の `{ ... }` の中身を字句解析し、元の位置を保った合成トークンを
/// そのトークンの直後に挿入する。以降は通常のコードと同じ経路で名前が解決される。
fn expand_template_tokens(tokens: Vec<Token>) -> Vec<Token> {
    let mut expanded = Vec::with_capacity(tokens.len());
    for token in tokens {
        let inner = if token.kind == TokenKind::TemplateString {
            template_expression_tokens(&token)
        } else {
            Vec::new()
        };
        expanded.push(token);
        expanded.extend(inner);
    }
    expanded
}

/// テンプレート文字列 `token` の埋め込み式のトークン。位置は元のソースのもの。
fn template_expression_tokens(token: &Token) -> Vec<Token> {
    let mut result = Vec::new();
    // 本文は逆引用符の次の文字から始まる。
    let (mut line, mut column) = (token.line, token.column + 1);
    let mut escaped = false;
    let mut depth = 0usize;
    let mut text = String::new();
    let mut start = (line, column);
    for ch in token.value.chars() {
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += ch.len_utf16();
        }
        if depth == 0 {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '{' {
                depth = 1;
                text.clear();
                // 式は `{` の次の文字から始まる。
                start = (line, column);
            }
            continue;
        }
        match ch {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            result.extend(lex_embedded(&text, start));
            continue;
        }
        text.push(ch);
    }
    result
}

/// 埋め込み式 `text`(元のソースの `start` から始まる)を字句解析し、元の位置のトークンにする。
fn lex_embedded(text: &str, start: (usize, usize)) -> Vec<Token> {
    let Ok(tokens) = Lexer::new(text).tokenize() else {
        return Vec::new();
    };
    let place = |line: usize, column: usize| {
        if line == 1 {
            (start.0, start.1 + column - 1)
        } else {
            (start.0 + line - 1, column)
        }
    };
    tokens
        .into_iter()
        .filter(|token| token.kind != TokenKind::Eof)
        .map(|token| {
            let (line, column) = place(token.line, token.column);
            let (end_line, end_column) = place(token.end_line, token.end_column);
            Token {
                line,
                column,
                end_line,
                end_column,
                ..token
            }
        })
        .collect()
}

fn run(tokens: Vec<Token>, bodyless_methods: bool, builtins: Builtins) -> TokenAnalysis {
    let tokens = expand_template_tokens(tokens);
    let count = tokens.len();
    let mut analyzer = Analyzer {
        tokens,
        entries: vec![Entry::default(); count],
        frames: vec![Frame::new(FrameKind::Global)],
        loop_header: false,
        underflow: false,
        bodyless_methods,
        class_members: HashMap::new(),
        class_decls: HashMap::new(),
        class_parents: HashMap::new(),
        field_keys: Vec::new(),
        top_decls: HashMap::new(),
        template_params: Vec::new(),
        template_depth: 0,
        builtins,
    };
    let mut index = 0;
    while index < count {
        index = analyzer.step(index);
    }
    let balanced = analyzer.frames.len() == 1 && !analyzer.underflow;
    TokenAnalysis {
        tokens: analyzer.tokens,
        entries: analyzer.entries,
        class_members: analyzer.class_members,
        class_decls: analyzer.class_decls,
        class_parents: analyzer.class_parents,
        field_keys: analyzer.field_keys,
        top_decls: analyzer.top_decls,
        balanced,
    }
}

impl Analyzer {
    fn kind_at(&self, index: usize) -> TokenKind {
        self.tokens
            .get(index)
            .map(|token| token.kind.clone())
            .unwrap_or(TokenKind::Eof)
    }

    fn prev_kind(&self, index: usize) -> Option<TokenKind> {
        index.checked_sub(1).map(|i| self.tokens[i].kind.clone())
    }

    fn is_ident_value(&self, index: usize, value: &str) -> bool {
        self.tokens
            .get(index)
            .is_some_and(|token| token.kind == TokenKind::Ident && token.value == value)
    }

    fn top(&self) -> FrameKind {
        self.frames.last().map_or(FrameKind::Global, |frame| frame.kind)
    }

    fn at_class_level(&self) -> bool {
        matches!(self.top(), FrameKind::Class | FrameKind::Members)
    }

    fn current_class(&self) -> Option<String> {
        self.frames
            .iter()
            .rev()
            .find(|frame| frame.kind == FrameKind::Class)
            .and_then(|frame| frame.class.clone())
    }

    /// 名前を宣言できる最も内側のスコープの番号。
    fn scope_index(&self) -> usize {
        self.frames
            .iter()
            .rposition(Frame::holds_names)
            .unwrap_or(0)
    }

    fn declare(&mut self, index: usize, kind: SymbolKind, readonly: bool) {
        self.declare_in(self.scope_index(), index, kind, readonly);
    }

    fn declare_in(&mut self, frame: usize, index: usize, kind: SymbolKind, readonly: bool) {
        let name = self.tokens[index].value.clone();
        self.frames[frame].names.insert(
            name,
            Decl {
                kind,
                token: index,
                readonly,
            },
        );
        self.entries[index] = Entry {
            kind: Some(kind),
            declaration: true,
            readonly,
            decl: Some(index),
            ..Entry::default()
        };
    }

    fn lookup(&self, name: &str) -> Option<Decl> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.names.get(name).copied())
    }

    fn mark(&mut self, index: usize, kind: SymbolKind) {
        self.entries[index].kind = Some(kind);
    }

    fn resolve(&mut self, index: usize) {
        let name = self.tokens[index].value.clone();
        let followed_by_call = self.kind_at(index + 1) == TokenKind::LParen;
        match self.lookup(&name) {
            Some(decl) => {
                self.entries[index] = Entry {
                    kind: Some(decl.kind),
                    readonly: decl.readonly,
                    decl: Some(decl.token),
                    ..Entry::default()
                };
            }
            None if self.builtins.globals.contains_key(&name) => {
                let kind = self.builtins.globals[&name];
                self.entries[index] = Entry {
                    kind: Some(match kind {
                        BuiltinKind::Function => SymbolKind::Function,
                        BuiltinKind::Namespace => SymbolKind::Namespace,
                        BuiltinKind::Variable => SymbolKind::Variable,
                    }),
                    readonly: kind == BuiltinKind::Variable,
                    builtin: true,
                    ..Entry::default()
                };
            }
            None => {
                self.entries[index] = Entry {
                    kind: Some(if followed_by_call {
                        SymbolKind::Function
                    } else {
                        SymbolKind::Variable
                    }),
                    unresolved: true,
                    ..Entry::default()
                };
            }
        }
    }

    fn pop(&mut self) {
        if self.frames.len() > 1 {
            self.frames.pop();
        } else {
            self.underflow = true;
        }
        self.end_template_if_closed();
    }

    fn pop_if(&mut self, kind: FrameKind) {
        if self.top() == kind {
            self.frames.pop();
        }
        self.end_template_if_closed();
    }

    /// `template <T>` を付けた関数・クラスが閉じたら、型引数の見える範囲を終える。
    fn end_template_if_closed(&mut self) {
        if !self.template_params.is_empty() && self.frames.len() <= self.template_depth {
            self.template_params.clear();
        }
    }

    fn step(&mut self, index: usize) -> usize {
        match self.kind_at(index) {
            TokenKind::Class => self.class_decl(index),
            TokenKind::Public | TokenKind::Private => {
                self.frames.push(Frame::new(FrameKind::Members));
                index + 1
            }
            TokenKind::Function => self.function_decl(index),
            TokenKind::Local => self.local_decl(index),
            TokenKind::For => self.for_decl(index),
            TokenKind::While => {
                self.frames.push(Frame::new(FrameKind::Block));
                self.loop_header = true;
                index + 1
            }
            TokenKind::If => {
                self.frames.push(Frame::new(FrameKind::Block));
                index + 1
            }
            TokenKind::Do => {
                if self.loop_header {
                    self.loop_header = false;
                } else {
                    self.frames.push(Frame::new(FrameKind::Block));
                }
                index + 1
            }
            TokenKind::Repeat => {
                self.frames.push(Frame::new(FrameKind::Repeat));
                index + 1
            }
            TokenKind::Until => {
                self.pop_if(FrameKind::Repeat);
                index + 1
            }
            // 本体を持たない `function f() abstract` は `end` で閉じない。
            TokenKind::Abstract => {
                if self.top() == FrameKind::Function {
                    self.frames.pop();
                }
                index + 1
            }
            TokenKind::End => {
                self.pop();
                index + 1
            }
            TokenKind::LBrace => {
                self.frames.push(Frame::new(FrameKind::Brace));
                index + 1
            }
            TokenKind::LParen => {
                self.frames.push(Frame::new(FrameKind::Paren));
                index + 1
            }
            TokenKind::LBracket => {
                self.frames.push(Frame::new(FrameKind::Bracket));
                index + 1
            }
            TokenKind::RBrace => {
                self.pop_if(FrameKind::Brace);
                index + 1
            }
            TokenKind::RParen => {
                self.pop_if(FrameKind::Paren);
                index + 1
            }
            TokenKind::RBracket => {
                self.pop_if(FrameKind::Bracket);
                index + 1
            }
            TokenKind::Goto => {
                // ラベルは別の名前空間。色付けも解決もしない。
                if self.kind_at(index + 1) == TokenKind::Ident {
                    index + 2
                } else {
                    index + 1
                }
            }
            TokenKind::DoubleColon => {
                if self.kind_at(index + 1) == TokenKind::Ident
                    && self.kind_at(index + 2) == TokenKind::DoubleColon
                {
                    index + 3
                } else {
                    // `expr :: Type` のキャスト。
                    self.mark_type(index + 1)
                }
            }
            TokenKind::Import => self.import_decl(index),
            TokenKind::Declare => self.declare_decl(index),
            TokenKind::Ident => self.ident(index),
            _ => index + 1,
        }
    }

    fn import_decl(&mut self, index: usize) -> usize {
        // import type Name
        if self.is_ident_value(index + 1, "type") && self.kind_at(index + 2) == TokenKind::Ident {
            self.mark(index + 1, SymbolKind::Keyword);
            self.declare_in(0, index + 2, SymbolKind::Namespace, false);
            // `import type Name from "path"`
            if self.is_ident_value(index + 3, "from") && self.kind_at(index + 4) == TokenKind::LuaString {
                self.mark(index + 3, SymbolKind::Keyword);
                return index + 5;
            }
            return index + 3;
        }
        index + 1
    }

    fn declare_decl(&mut self, index: usize) -> usize {
        let mut next = index + 1;
        if self.kind_at(next) == TokenKind::Class {
            return next;
        }
        if self.kind_at(next) == TokenKind::Global {
            next += 1;
        }
        if self.kind_at(next) == TokenKind::Function {
            return self.declared_function(next);
        }
        if self.kind_at(next) == TokenKind::Ident {
            let name = self.tokens[next].value.clone();
            self.declare_in(0, next, SymbolKind::Variable, false);
            self.top_decls.insert(name, next);
            return next + 1;
        }
        next
    }

    /// `declare [global] function name(params)[: Return]`。`function` の位置から読む。
    fn declared_function(&mut self, index: usize) -> usize {
        let mut next = index + 1;
        if self.kind_at(next) == TokenKind::Ident {
            let name = self.tokens[next].value.clone();
            self.declare_in(0, next, SymbolKind::Function, false);
            self.top_decls.insert(name, next);
            next += 1;
        }
        if self.kind_at(next) == TokenKind::LParen {
            next += 1;
            loop {
                match self.kind_at(next) {
                    TokenKind::Ident => {
                        self.entries[next] = Entry {
                            kind: Some(SymbolKind::Parameter),
                            declaration: true,
                            decl: Some(next),
                            ..Entry::default()
                        };
                        next += 1;
                        if self.kind_at(next) == TokenKind::Colon {
                            next = self.mark_type(next + 1);
                        }
                    }
                    TokenKind::DotDotDot | TokenKind::Comma => next += 1,
                    _ => break,
                }
            }
            if self.kind_at(next) == TokenKind::RParen {
                next += 1;
            }
        }
        if self.kind_at(next) == TokenKind::Colon {
            next = self.mark_type(next + 1);
        }
        self.template_params.clear();
        next
    }

    /// `template <T, U>`。型引数を、直後の宣言の中だけで見える型として宣言する。
    fn template_header(&mut self, index: usize) -> usize {
        self.mark(index, SymbolKind::Keyword);
        self.template_params.clear();
        self.template_depth = self.frames.len();
        let mut next = index + 2;
        while self.kind_at(next) == TokenKind::Ident {
            let name = self.tokens[next].value.clone();
            self.entries[next] = Entry {
                kind: Some(SymbolKind::Type),
                declaration: true,
                decl: Some(next),
                ..Entry::default()
            };
            self.template_params.push((name, next));
            next += 1;
            if self.kind_at(next) == TokenKind::Comma {
                next += 1;
            } else {
                break;
            }
        }
        if self.kind_at(next) == TokenKind::Gt {
            next += 1;
        }
        next
    }

    /// `[export] type Name = TypeExpr`。`type` / `export` の位置から読む。
    fn type_alias_decl(&mut self, index: usize) -> usize {
        let mut next = index;
        if self.is_ident_value(next, "export") {
            self.mark(next, SymbolKind::Keyword);
            next += 1;
        }
        self.mark(next, SymbolKind::Keyword);
        next += 1;
        self.declare_in(0, next, SymbolKind::Type, false);
        next += 2; // 名前と `=`
        let next = self.mark_type(next);
        self.template_params.clear();
        next
    }

    fn starts_type_alias(&self, index: usize) -> bool {
        let at = if self.is_ident_value(index, "export") { index + 1 } else { index };
        self.is_ident_value(at, "type")
            && self.kind_at(at + 1) == TokenKind::Ident
            && self.kind_at(at + 2) == TokenKind::Eq
    }

    /// 型注釈を1つ読み、型名を色付けする。次のtoken番号を返す。
    fn mark_type(&mut self, start: usize) -> usize {
        let mut index = start;
        match self.kind_at(index) {
            TokenKind::LParen => {
                index = self.mark_type_list(index + 1, TokenKind::RParen);
                if self.kind_at(index) == TokenKind::RParen {
                    index += 1;
                }
                if self.kind_at(index) == TokenKind::Arrow {
                    return self.mark_type(index + 1);
                }
            }
            TokenKind::LBrace => index = self.mark_table_type(index),
            TokenKind::Ident => index = self.mark_named_type(index),
            TokenKind::Nil => index += 1,
            _ => return index,
        }
        if self.kind_at(index) == TokenKind::Question {
            index += 1;
        }
        // ユニオン `A | B`。
        if self.kind_at(index) == TokenKind::Pipe {
            return self.mark_type(index + 1);
        }
        index
    }

    /// `close` の手前まで、カンマ区切りの型を読む。
    fn mark_type_list(&mut self, start: usize, close: TokenKind) -> usize {
        let mut index = start;
        while self.kind_at(index) != close && self.kind_at(index) != TokenKind::Eof {
            let next = self.mark_type(index);
            if next == index {
                break;
            }
            index = next;
            if self.kind_at(index) == TokenKind::Comma {
                index += 1;
            } else {
                break;
            }
        }
        index
    }

    /// `Name`、`mod.Name`、`Name<A, B>`。
    fn mark_named_type(&mut self, start: usize) -> usize {
        let mut index = start;
        if self.kind_at(index + 1) == TokenKind::Dot && self.kind_at(index + 2) == TokenKind::Ident {
            // `mod.Name`: 前半はモジュール、後半が型名。
            self.resolve(index);
            self.entries[index + 2] = Entry {
                kind: Some(SymbolKind::Type),
                ..Entry::default()
            };
            index += 3;
        } else {
            self.mark_type_reference(index);
            index += 1;
        }
        if self.kind_at(index) == TokenKind::Lt {
            index = self.mark_type_list(index + 1, TokenKind::Gt);
            if self.kind_at(index) == TokenKind::Gt {
                index += 1;
            }
        }
        index
    }

    /// `{ id: number, ref: T }`
    fn mark_table_type(&mut self, start: usize) -> usize {
        let mut index = start + 1;
        // `{ T }`: 配列型。
        let is_field = self.kind_at(index) == TokenKind::Ident
            && self.kind_at(index + 1) == TokenKind::Colon;
        if !is_field && self.kind_at(index) != TokenKind::RBrace {
            index = self.mark_type(index);
        }
        while self.kind_at(index) == TokenKind::Ident && self.kind_at(index + 1) == TokenKind::Colon {
            self.entries[index] = Entry {
                kind: Some(SymbolKind::Property),
                ..Entry::default()
            };
            index = self.mark_type(index + 2);
            if self.kind_at(index) == TokenKind::Comma {
                index += 1;
            } else {
                break;
            }
        }
        if self.kind_at(index) == TokenKind::RBrace {
            index += 1;
        }
        index
    }

    /// 型注釈の位置の名前。クラス・別名・型引数も、プリミティブと同じ `type` で色付けする。
    fn mark_type_reference(&mut self, index: usize) {
        self.mark_type_name(index);
        if self.entries[index].kind == Some(SymbolKind::Class) {
            self.entries[index].kind = Some(SymbolKind::Type);
        }
    }

    fn mark_type_name(&mut self, index: usize) {
        let name = self.tokens[index].value.clone();
        if PRIMITIVE_TYPES.contains(&name.as_str()) {
            self.entries[index] = Entry {
                kind: Some(SymbolKind::Type),
                ..Entry::default()
            };
            return;
        }
        if let Some((_, token)) = self.template_params.iter().rev().find(|(param, _)| *param == name) {
            self.entries[index] = Entry {
                kind: Some(SymbolKind::Type),
                decl: Some(*token),
                ..Entry::default()
            };
            return;
        }
        let found = self
            .lookup(&name)
            .filter(|decl| matches!(decl.kind, SymbolKind::Class | SymbolKind::Type));
        self.entries[index] = Entry {
            kind: Some(found.map_or(SymbolKind::Class, |decl| decl.kind)),
            decl: found.map(|decl| decl.token),
            unresolved: found.is_none(),
            ..Entry::default()
        };
    }

    fn class_decl(&mut self, index: usize) -> usize {
        let scope = self.scope_index();
        let mut next = index + 1;
        if self.kind_at(next) == TokenKind::Abstract {
            next += 1;
        }
        let mut frame = Frame::new(FrameKind::Class);
        if self.kind_at(next) == TokenKind::Ident {
            let name = self.tokens[next].value.clone();
            self.declare_in(scope, next, SymbolKind::Class, false);
            self.class_decls.insert(name.clone(), next);
            self.class_members.entry(name.clone()).or_default();
            frame.class = Some(name.clone());
            next += 1;
            if self.kind_at(next) == TokenKind::Is {
                let header_line = self.tokens[next].line;
                next += 1;
                if self.kind_at(next) == TokenKind::Abstract {
                    next += 1;
                }
                if self.kind_at(next) == TokenKind::Ident && self.tokens[next].line == header_line {
                    let parent = self.tokens[next].value.clone();
                    self.mark_type_name(next);
                    self.class_parents.insert(name, parent);
                    next += 1;
                }
            }
        }
        self.frames.push(frame);
        next
    }

    fn local_decl(&mut self, index: usize) -> usize {
        if self.kind_at(index + 1) == TokenKind::Function {
            return index + 1;
        }
        self.declare_names(index + 1, SymbolKind::Variable, false)
    }

    fn for_decl(&mut self, index: usize) -> usize {
        self.frames.push(Frame::new(FrameKind::Block));
        self.loop_header = true;
        self.declare_names(index + 1, SymbolKind::Variable, false)
    }

    /// `a, b: T, c` の形の名前列を宣言する。次のtoken番号を返す。
    fn declare_names(&mut self, start: usize, kind: SymbolKind, readonly: bool) -> usize {
        let mut index = start;
        while self.kind_at(index) == TokenKind::Ident {
            self.declare(index, kind, readonly);
            index += 1;
            if self.kind_at(index) == TokenKind::Colon {
                index = self.mark_type(index + 1);
            }
            if self.kind_at(index) == TokenKind::Comma {
                index += 1;
            } else {
                break;
            }
        }
        index
    }

    fn function_decl(&mut self, index: usize) -> usize {
        let owner_is_class = self.at_class_level();
        let is_static = self.prev_kind(index) == Some(TokenKind::Static);
        let readonly = index > 0 && self.is_ident_value(index - 1, "const");
        let scope = self.scope_index();
        let class = self.current_class();

        let mut next = index + 1;
        if self.kind_at(next) == TokenKind::Operator {
            // `function operator==(`
            next += 2;
        } else if self.kind_at(next) == TokenKind::Ident {
            let mut chain = vec![next];
            next += 1;
            while matches!(self.kind_at(next), TokenKind::Dot | TokenKind::Colon)
                && self.kind_at(next + 1) == TokenKind::Ident
            {
                chain.push(next + 1);
                next += 2;
            }
            if chain.len() == 1 {
                let name_index = chain[0];
                if owner_is_class {
                    self.declare_member(name_index, class.as_deref(), SymbolKind::Method, is_static);
                } else {
                    self.declare_in(scope, name_index, SymbolKind::Function, readonly);
                }
            } else {
                self.resolve(chain[0]);
                for &middle in &chain[1..chain.len() - 1] {
                    self.entries[middle] = Entry {
                        kind: Some(SymbolKind::Property),
                        is_member: true,
                        ..Entry::default()
                    };
                }
                let last = *chain.last().expect("non-empty chain");
                let name = self.tokens[last].value.clone();
                self.field_keys.push((name, last));
                self.entries[last] = Entry {
                    kind: Some(SymbolKind::Method),
                    declaration: true,
                    decl: Some(last),
                    ..Entry::default()
                };
            }
        }

        if !self.bodyless_methods || !owner_is_class {
            self.frames.push(Frame::new(FrameKind::Function));
        } else {
            // 本体なしのメソッドでも、引数のために一時的なスコープを使う。
            self.frames.push(Frame::new(FrameKind::Function));
        }
        let function_frame = self.frames.len() - 1;

        if self.kind_at(next) == TokenKind::LParen {
            next += 1;
            loop {
                match self.kind_at(next) {
                    TokenKind::Ident => {
                        self.declare_in(function_frame, next, SymbolKind::Parameter, false);
                        next += 1;
                        if self.kind_at(next) == TokenKind::Colon {
                            next = self.mark_type(next + 1);
                        }
                    }
                    TokenKind::DotDotDot | TokenKind::Comma => next += 1,
                    // メソッドの先頭の `self`(と添えた型)。
                    TokenKind::Self_ => {
                        next += 1;
                        if self.kind_at(next) == TokenKind::Colon {
                            next = self.mark_type(next + 1);
                        }
                    }
                    _ => break,
                }
            }
            if self.kind_at(next) == TokenKind::RParen {
                next += 1;
            }
        }
        // 戻り値の型注釈。
        if self.kind_at(next) == TokenKind::Colon {
            next = self.mark_type(next + 1);
        }
        if self.bodyless_methods && owner_is_class {
            // 宣言ファイルのメソッドは `end` を持たないので、署名の直後で閉じる。
            self.frames.pop();
            self.end_template_if_closed();
        }
        next
    }

    fn declare_member(
        &mut self,
        index: usize,
        class: Option<&str>,
        kind: SymbolKind,
        is_static: bool,
    ) {
        self.entries[index] = Entry {
            kind: Some(kind),
            declaration: true,
            is_static,
            decl: Some(index),
            ..Entry::default()
        };
        if let Some(class) = class {
            let name = self.tokens[index].value.clone();
            self.class_members
                .entry(class.to_string())
                .or_default()
                .push((name, index));
        }
    }

    fn ident(&mut self, index: usize) -> usize {
        let previous = self.prev_kind(index);
        let next = self.kind_at(index + 1);

        // `template <T, U>`
        if self.tokens[index].value == "template"
            && next == TokenKind::Lt
            && self.kind_at(index + 2) == TokenKind::Ident
            && !matches!(previous, Some(TokenKind::Dot | TokenKind::Colon))
        {
            return self.template_header(index);
        }
        // `[export] type Name = ...`
        if self.starts_type_alias(index) && !matches!(previous, Some(TokenKind::Dot | TokenKind::Colon)) {
            return self.type_alias_decl(index);
        }
        // `friend class X`
        if self.tokens[index].value == "friend"
            && next == TokenKind::Class
            && self.at_class_level()
            && self.kind_at(index + 2) == TokenKind::Ident
        {
            self.mark(index, SymbolKind::Keyword);
            self.mark_type_name(index + 2);
            return index + 3;
        }
        // `const NAME` / `const function`
        if self.tokens[index].value == "const"
            && matches!(next, TokenKind::Ident | TokenKind::Function)
            && !matches!(previous, Some(TokenKind::Dot | TokenKind::Colon))
        {
            self.mark(index, SymbolKind::Keyword);
            if next == TokenKind::Ident {
                return self.declare_names(index + 1, SymbolKind::Variable, true);
            }
            return index + 1;
        }

        // `x.name` / `x:name(...)`
        let is_method_call = previous == Some(TokenKind::Colon) && next == TokenKind::LParen;
        if previous == Some(TokenKind::Dot) || is_method_call {
            self.member_use(index, next == TokenKind::LParen);
            return index + 1;
        }
        // `if name := value`
        if next == TokenKind::Bind {
            self.declare(index, SymbolKind::Variable, false);
            return index + 1;
        }
        // `{ name = value }`
        if self.top() == FrameKind::Brace
            && matches!(
                previous,
                Some(TokenKind::LBrace | TokenKind::Comma | TokenKind::Semicolon)
            )
            && next == TokenKind::Eq
        {
            let name = self.tokens[index].value.clone();
            self.field_keys.push((name, index));
            self.entries[index] = Entry {
                kind: Some(SymbolKind::Property),
                declaration: true,
                decl: Some(index),
                ..Entry::default()
            };
            return index + 1;
        }
        // クラス直下のフィールド宣言: `name = ...` / `name: Type`
        if self.at_class_level()
            && matches!(next, TokenKind::Eq | TokenKind::Colon)
            && !matches!(previous, Some(TokenKind::Eq | TokenKind::Colon))
        {
            let class = self.current_class();
            self.declare_member(index, class.as_deref(), SymbolKind::Property, false);
            if next == TokenKind::Colon {
                return self.mark_type(index + 2);
            }
            return index + 1;
        }
        // 型注釈: `: Type`
        if previous == Some(TokenKind::Colon) {
            return self.mark_type(index);
        }

        self.resolve(index);
        index + 1
    }

    fn member_use(&mut self, index: usize, followed_by_call: bool) {
        let name = self.tokens[index].value.clone();
        let assigned = self.kind_at(index + 1) == TokenKind::Eq;
        // `self.x = ...` は、そのクラスのフィールドの定義になる。
        let on_self = index >= 2 && self.tokens[index - 2].kind == TokenKind::Self_;
        if assigned {
            self.field_keys.push((name.clone(), index));
            if on_self {
                if let Some(class) = self.current_class() {
                    let known = self
                        .class_members
                        .get(&class)
                        .is_some_and(|members| members.iter().any(|(member, _)| *member == name));
                    if !known {
                        self.declare_member(index, Some(&class), SymbolKind::Property, false);
                        self.entries[index].is_member = true;
                        return;
                    }
                }
            }
        }
        if !assigned && self.mark_builtin_member(index) {
            return;
        }
        self.entries[index] = Entry {
            kind: Some(if followed_by_call {
                SymbolKind::Method
            } else {
                SymbolKind::Property
            }),
            is_member: true,
            ..Entry::default()
        };
    }

    /// `math.floor` / `math.pi` のように、標準の名前空間のメンバーなら標準の名前として色付けする。
    fn mark_builtin_member(&mut self, index: usize) -> bool {
        if index < 2 || !self.entries[index - 2].builtin {
            return false;
        }
        let namespace = self.tokens[index - 2].value.clone();
        let member = self.tokens[index].value.clone();
        let Some(kind) = self.builtins.member(&namespace, &member) else {
            return false;
        };
        self.entries[index] = Entry {
            kind: Some(match kind {
                BuiltinKind::Function => SymbolKind::Function,
                _ => SymbolKind::Variable,
            }),
            readonly: kind != BuiltinKind::Function,
            builtin: true,
            ..Entry::default()
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn analyze_source(source: &str) -> TokenAnalysis {
        analyze(Lexer::new(source).tokenize().unwrap())
    }

    fn kind_of(analysis: &TokenAnalysis, line: usize, column: usize) -> Option<(SymbolKind, bool)> {
        analysis
            .tokens
            .iter()
            .zip(&analysis.entries)
            .find(|(token, _)| token.line == line && token.column == column)
            .and_then(|(_, entry)| entry.kind.map(|kind| (kind, entry.declaration)))
    }

    #[test]
    fn locals_consts_functions_and_parameters_are_distinguished() {
        let analysis = analyze_source(
            "local count: number = 1\nconst NAME = \"x\"\nfunction go(arg)\n    print(count, NAME, arg)\nend\ngo(1)",
        );
        assert_eq!(kind_of(&analysis, 1, 7), Some((SymbolKind::Variable, true)));
        assert_eq!(kind_of(&analysis, 1, 14), Some((SymbolKind::Type, false)));
        assert_eq!(kind_of(&analysis, 2, 1), Some((SymbolKind::Keyword, false)));
        assert_eq!(kind_of(&analysis, 2, 7), Some((SymbolKind::Variable, true)));
        assert_eq!(kind_of(&analysis, 3, 10), Some((SymbolKind::Function, true)));
        assert_eq!(kind_of(&analysis, 3, 13), Some((SymbolKind::Parameter, true)));
        assert_eq!(kind_of(&analysis, 4, 5), Some((SymbolKind::Function, false)));
        assert_eq!(kind_of(&analysis, 4, 11), Some((SymbolKind::Variable, false)));
        assert_eq!(kind_of(&analysis, 4, 18), Some((SymbolKind::Variable, false)));
        assert_eq!(kind_of(&analysis, 4, 24), Some((SymbolKind::Parameter, false)));
        assert_eq!(kind_of(&analysis, 6, 1), Some((SymbolKind::Function, false)));
        let entry = analysis
            .tokens
            .iter()
            .zip(&analysis.entries)
            .find(|(token, _)| token.line == 4 && token.column == 18)
            .unwrap()
            .1;
        assert!(entry.readonly);
        assert_eq!(analysis.tokens[entry.decl.unwrap()].line, 2);
    }

    #[test]
    fn class_members_methods_and_member_access() {
        let analysis = analyze_source(
            "class Dog is\n    public is\n        name = \"x\"\n        static function create()\n        end\n        function bark(times: number)\n            self.age = 1\n        end\n    end\nend\nlocal d = Dog.new()\nd.bark(1)\nprint(d.name)",
        );
        assert!(analysis.balanced);
        assert_eq!(kind_of(&analysis, 1, 7), Some((SymbolKind::Class, true)));
        assert_eq!(kind_of(&analysis, 3, 9), Some((SymbolKind::Property, true)));
        assert_eq!(kind_of(&analysis, 4, 25), Some((SymbolKind::Method, true)));
        assert_eq!(kind_of(&analysis, 6, 18), Some((SymbolKind::Method, true)));
        assert_eq!(kind_of(&analysis, 11, 11), Some((SymbolKind::Class, false)));
        assert_eq!(kind_of(&analysis, 11, 15), Some((SymbolKind::Method, false)));
        assert_eq!(kind_of(&analysis, 12, 3), Some((SymbolKind::Method, false)));
        assert_eq!(kind_of(&analysis, 13, 9), Some((SymbolKind::Property, false)));
        let members: Vec<_> = analysis.class_members["Dog"]
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(members, vec!["name", "create", "bark", "age"]);
    }

    #[test]
    fn shadowing_and_scopes_resolve_to_the_nearest_declaration() {
        let analysis = analyze_source(
            "local x = 1\ndo\n    local x = 2\n    print(x)\nend\nprint(x)",
        );
        let decl_line = |line: usize, column: usize| {
            let (_, entry) = analysis
                .tokens
                .iter()
                .zip(&analysis.entries)
                .find(|(token, _)| token.line == line && token.column == column)
                .unwrap();
            analysis.tokens[entry.decl.unwrap()].line
        };
        assert_eq!(decl_line(4, 11), 3);
        assert_eq!(decl_line(6, 7), 1);
    }

    #[test]
    fn bindings_loops_import_and_table_keys() {
        let analysis = analyze_source(
            "import type ext\nfor i = 1, 3 do print(i) end\nif found := lookup() then print(found) end\nlocal t = { key = 1 }\nprint(unknown)",
        );
        assert_eq!(kind_of(&analysis, 1, 13), Some((SymbolKind::Namespace, true)));
        assert_eq!(kind_of(&analysis, 2, 5), Some((SymbolKind::Variable, true)));
        assert_eq!(kind_of(&analysis, 3, 4), Some((SymbolKind::Variable, true)));
        assert_eq!(kind_of(&analysis, 4, 13), Some((SymbolKind::Property, true)));
        assert_eq!(analysis.field_keys[0].0, "key");
        let unknown = analysis
            .tokens
            .iter()
            .zip(&analysis.entries)
            .find(|(token, _)| token.value == "unknown")
            .unwrap()
            .1;
        assert!(unknown.unresolved);
    }

    #[test]
    fn definition_files_use_bodyless_methods_and_top_level_declarations() {
        let tokens = Lexer::new(
            "declare class Part is\n    public is\n        Anchored = false\n    end\n    private is\n        static function new(): Part\n    end\nend\ndeclare dog: Part\n",
        )
        .tokenize()
        .unwrap();
        let analysis = analyze_definition(tokens);
        assert!(analysis.balanced);
        assert!(analysis.top_decls.contains_key("dog"));
        let members: Vec<_> = analysis.class_members["Part"]
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(members, vec!["Anchored", "new"]);
        // `dog: Part` の `Part` は同じファイルのクラスへ解決され、型の位置なので `type` で色付けする。
        let part_ref = analysis
            .tokens
            .iter()
            .zip(&analysis.entries)
            .filter(|(token, _)| token.value == "Part" && token.line == 9)
            .next()
            .unwrap()
            .1;
        assert_eq!(part_ref.kind, Some(SymbolKind::Type));
        assert!(part_ref.decl.is_some());
    }

    #[test]
    fn definition_files_with_method_bodies_are_still_balanced() {
        let tokens = Lexer::new(
            "declare class Part is\n    public is\n        static function make()\n            if true then\n                return 1\n            end\n        end\n    end\nend\n",
        )
        .tokenize()
        .unwrap();
        assert!(analyze_definition(tokens).balanced);
    }
}
