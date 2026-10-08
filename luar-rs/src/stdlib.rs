//! Lua 5.4 / Luau の標準ライブラリの型つき宣言。
//!
//! `.luard` と同じ文法のテキストで持ち、`modules::parse_definition` で読む。チェッカーは
//! これを最初の型環境に入れ、色分けは同じ定義から標準名とそのメンバーを引く。
//! 多値の戻り値とオーバーロードは `any` と可変長で逃がす(最初の値の型だけを持つ)。

use crate::Target;
use crate::ast::TypeExpr;
use crate::modules::{self, ModuleDefinition};
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

/// Lua 5.4 と Luau に共通する部分。
const COMMON: &str = r#"
declare global _G: table
declare global _VERSION: string
declare global function assert(value: any, ...): any
declare global function collectgarbage(option: string?, arg: number?): any
declare global function error(message: any, level: number?)
declare global function getmetatable(object: any): any
declare global function ipairs(t: table): any
declare global function next(t: table, key: any?): any
declare global function pairs(t: table): any
declare global function pcall(f: any, ...): boolean
declare global function print(...)
declare global function rawequal(a: any, b: any): boolean
declare global function rawget(t: table, key: any): any
declare global function rawlen(value: any): number
declare global function rawset(t: table, key: any, value: any): table
declare global function require(path: any): any
declare global function select(index: any, ...): any
declare global function setmetatable(t: table, metatable: table?): table
declare global function tonumber(value: any, base: number?): number?
declare global function tostring(value: any): string
declare global function type(value: any): string
declare global function xpcall(f: any, handler: any, ...): boolean

declare global string: {
    byte: (string, number?, number?) -> number,
    char: (...number) -> string,
    find: (string, string, number?, boolean?) -> number?,
    format: (string, ...any) -> string,
    gmatch: (string, string) -> any,
    gsub: (string, string, any, number?) -> string,
    len: (string) -> number,
    lower: (string) -> string,
    match: (string, string, number?) -> string?,
    pack: (string, ...any) -> string,
    packsize: (string) -> number,
    rep: (string, number, string?) -> string,
    reverse: (string) -> string,
    sub: (string, number, number?) -> string,
    unpack: (string, string, number?) -> any,
    upper: (string) -> string
}

declare global table: {
    concat: (table, string?, number?, number?) -> string,
    insert: (table, any, ...any) -> (),
    move: (table, number, number, number, table?) -> table,
    pack: (...any) -> table,
    remove: (table, number?) -> any,
    sort: (table, any?) -> (),
    unpack: (table, number?, number?) -> any
}

declare global os: {
    clock: () -> number,
    date: (string?, number?) -> any,
    difftime: (number, number) -> number,
    time: (table?) -> number
}

declare global utf8: {
    char: (...number) -> string,
    charpattern: string,
    codepoint: (string, number?, number?) -> number,
    codes: (string) -> any,
    len: (string, number?, number?) -> number?,
    offset: (string, number, number?) -> number?
}

declare global coroutine: {
    close: (any) -> boolean,
    create: (any) -> any,
    isyieldable: () -> boolean,
    resume: (any, ...any) -> boolean,
    running: () -> any,
    status: (any) -> string,
    wrap: (any) -> any,
    yield: (...any) -> any
}

declare global debug: table
"#;

/// Lua 5.4 だけにあるもの。
const LUA54: &str = r#"
declare global function dofile(path: string?): any
declare global function load(chunk: any, chunkname: string?, mode: string?, env: table?): any
declare global function loadfile(path: string?, mode: string?, env: table?): any

declare global math: {
    abs: (number) -> number,
    acos: (number) -> number,
    asin: (number) -> number,
    atan: (number, number?) -> number,
    ceil: (number) -> number,
    cos: (number) -> number,
    deg: (number) -> number,
    exp: (number) -> number,
    floor: (number) -> number,
    fmod: (number, number) -> number,
    huge: number,
    log: (number, number?) -> number,
    max: (number, ...number) -> number,
    maxinteger: number,
    min: (number, ...number) -> number,
    mininteger: number,
    modf: (number) -> number,
    pi: number,
    rad: (number) -> number,
    random: (number?, number?) -> number,
    randomseed: (...number) -> (),
    sin: (number) -> number,
    sqrt: (number) -> number,
    tan: (number) -> number,
    tointeger: (any) -> number?,
    type: (any) -> string?,
    ult: (number, number) -> boolean
}

declare global io: {
    close: (any?) -> any,
    lines: (string?, ...any) -> any,
    open: (string, string?) -> any,
    read: (...any) -> any,
    stderr: any,
    stdin: any,
    stdout: any,
    write: (...any) -> any
}

declare global package: table
"#;

/// Luau だけにあるもの。
const LUAU: &str = r#"
declare global function gcinfo(): number
declare global function getfenv(level: any?): table
declare global function newproxy(addMetatable: boolean?): any
declare global function setfenv(target: any, env: table): any
declare global function typeof(value: any): string
declare global function warn(...)

declare global math: {
    abs: (number) -> number,
    acos: (number) -> number,
    asin: (number) -> number,
    atan: (number) -> number,
    atan2: (number, number) -> number,
    ceil: (number) -> number,
    clamp: (number, number, number) -> number,
    cos: (number) -> number,
    cosh: (number) -> number,
    deg: (number) -> number,
    exp: (number) -> number,
    floor: (number) -> number,
    fmod: (number, number) -> number,
    frexp: (number) -> number,
    huge: number,
    ldexp: (number, number) -> number,
    lerp: (number, number, number) -> number,
    log: (number, number?) -> number,
    log10: (number) -> number,
    map: (number, number, number, number, number) -> number,
    max: (number, ...number) -> number,
    min: (number, ...number) -> number,
    modf: (number) -> number,
    noise: (number, number?, number?) -> number,
    pi: number,
    pow: (number, number) -> number,
    rad: (number) -> number,
    random: (number?, number?) -> number,
    randomseed: (number) -> (),
    round: (number) -> number,
    sign: (number) -> number,
    sin: (number) -> number,
    sinh: (number) -> number,
    sqrt: (number) -> number,
    tan: (number) -> number,
    tanh: (number) -> number
}

declare global bit32: {
    arshift: (number, number) -> number,
    band: (...number) -> number,
    bnot: (number) -> number,
    bor: (...number) -> number,
    btest: (...number) -> boolean,
    bxor: (...number) -> number,
    byteswap: (number) -> number,
    countlz: (number) -> number,
    countrz: (number) -> number,
    extract: (number, number, number?) -> number,
    lrotate: (number, number) -> number,
    lshift: (number, number) -> number,
    replace: (number, number, number, number?) -> number,
    rrotate: (number, number) -> number,
    rshift: (number, number) -> number
}

declare global buffer: table

declare global task: {
    cancel: (any) -> (),
    defer: (any, ...any) -> any,
    delay: (number, any, ...any) -> any,
    spawn: (any, ...any) -> any,
    wait: (number?) -> number
}
"#;

/// Luau の `table` と `string` への追加。共通部の宣言を上書きする。
const LUAU_TABLE: &str = r#"
declare global table: {
    clear: (table) -> (),
    clone: (table) -> table,
    concat: (table, string?, number?, number?) -> string,
    create: (number, any?) -> table,
    find: (table, any, number?) -> number?,
    freeze: (table) -> table,
    getn: (table) -> number,
    insert: (table, any, ...any) -> (),
    isfrozen: (table) -> boolean,
    maxn: (table) -> number,
    move: (table, number, number, number, table?) -> table,
    pack: (...any) -> table,
    remove: (table, number?) -> any,
    sort: (table, any?) -> (),
    unpack: (table, number?, number?) -> any
}

declare global string: {
    byte: (string, number?, number?) -> number,
    char: (...number) -> string,
    find: (string, string, number?, boolean?) -> number?,
    format: (string, ...any) -> string,
    gmatch: (string, string) -> any,
    gsub: (string, string, any, number?) -> string,
    len: (string) -> number,
    lower: (string) -> string,
    match: (string, string, number?) -> string?,
    pack: (string, ...any) -> string,
    packsize: (string) -> number,
    rep: (string, number, string?) -> string,
    reverse: (string) -> string,
    split: (string, string?) -> table,
    sub: (string, number, number?) -> string,
    unpack: (string, string, number?) -> any,
    upper: (string) -> string
}
"#;

/// 指定ターゲットの標準ライブラリ。
pub fn definition(target: Target) -> &'static ModuleDefinition {
    static LUA54_DEFINITION: OnceLock<ModuleDefinition> = OnceLock::new();
    static LUAU_DEFINITION: OnceLock<ModuleDefinition> = OnceLock::new();
    match target {
        Target::Lua54 => LUA54_DEFINITION.get_or_init(|| build(&[COMMON, LUA54])),
        Target::Luau => LUAU_DEFINITION.get_or_init(|| build_luau()),
    }
}

fn build_luau() -> ModuleDefinition {
    // Luau の `table` / `string` は共通部の宣言と置き換える。
    let common = remove_declarations(COMMON, &["table", "string"]);
    build(&[&common, LUAU, LUAU_TABLE])
}

/// 先頭が `declare global <name>: {` で `}` で閉じる宣言を取り除く。
fn remove_declarations(source: &str, names: &[&str]) -> String {
    let mut output = String::new();
    let mut skipping = false;
    for line in source.lines() {
        if skipping {
            if line.trim_start().starts_with('}') {
                skipping = false;
            }
            continue;
        }
        let removed = names
            .iter()
            .any(|name| line.starts_with(&format!("declare global {name}: {{")));
        if removed {
            skipping = true;
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }
    output
}

fn build(parts: &[&str]) -> ModuleDefinition {
    let source = parts.join("\n");
    let mut definition = modules::parse_definition("", Path::new("<stdlib>"), &source)
        .unwrap_or_else(|error| panic!("invalid built-in library declarations: {}", error.message));
    definition.name = String::new();
    definition
}

/// 標準ライブラリの名前の種別。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BuiltinKind {
    Function,
    /// `math` のようなメンバーを持つ名前空間。
    Namespace,
    Variable,
}

/// 色分けのために、標準名とそのメンバーを引ける形にしたもの。
#[derive(Debug, Default)]
pub struct Builtins {
    pub globals: HashMap<String, BuiltinKind>,
    /// 名前空間 → (メンバー名, 種別)。
    pub members: HashMap<String, Vec<(String, BuiltinKind)>>,
}

impl Builtins {
    pub fn for_target(target: Target) -> Self {
        let definition = definition(target);
        let mut builtins = Builtins::default();
        for function in &definition.functions {
            builtins
                .globals
                .insert(function.name.clone(), BuiltinKind::Function);
        }
        for (name, ty) in &definition.global_types {
            match ty {
                TypeExpr::Table(fields) => {
                    builtins.globals.insert(name.clone(), BuiltinKind::Namespace);
                    let members = fields
                        .iter()
                        .map(|(member, field)| (member.clone(), kind_of(field)))
                        .collect();
                    builtins.members.insert(name.clone(), members);
                }
                other => {
                    builtins.globals.insert(name.clone(), kind_of(other));
                }
            }
        }
        builtins
    }

    pub fn member(&self, namespace: &str, member: &str) -> Option<BuiltinKind> {
        self.members
            .get(namespace)?
            .iter()
            .find(|(name, _)| name == member)
            .map(|(_, kind)| *kind)
    }
}

fn kind_of(ty: &TypeExpr) -> BuiltinKind {
    match ty {
        TypeExpr::Function { .. } => BuiltinKind::Function,
        TypeExpr::Optional(inner) => kind_of(inner),
        _ => BuiltinKind::Variable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarations_parse_for_every_target() {
        for target in [Target::Lua54, Target::Luau] {
            let builtins = Builtins::for_target(target);
            assert_eq!(builtins.globals.get("print"), Some(&BuiltinKind::Function));
            assert_eq!(builtins.globals.get("math"), Some(&BuiltinKind::Namespace));
            assert_eq!(builtins.member("math", "floor"), Some(BuiltinKind::Function));
            assert_eq!(builtins.member("math", "pi"), Some(BuiltinKind::Variable));
        }
    }

    #[test]
    fn targets_differ_in_what_they_provide() {
        let lua = Builtins::for_target(Target::Lua54);
        let luau = Builtins::for_target(Target::Luau);
        assert!(lua.globals.contains_key("io") && !luau.globals.contains_key("io"));
        assert!(luau.globals.contains_key("task") && !lua.globals.contains_key("task"));
        assert!(luau.member("table", "clone").is_some() && lua.member("table", "clone").is_none());
        assert!(lua.member("math", "tointeger").is_some() && luau.member("math", "tointeger").is_none());
    }
}
