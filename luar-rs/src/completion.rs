//! エディタ補完のための問い合わせ。
//!
//! 入力途中のソースは構文エラーになりやすいので、カーソル位置の式を
//! `__luar_probe(<receiver>)` / `__luar_scope()` という呼び出しへ書き換えて通常の
//! 解析に通す。チェッカーはその呼び出しに到達した時点の型環境を記録し、
//! それを候補へ変換する。型の判断は診断と同じチェッカーが行う。

use crate::ast::{Param, TypeExpr};
use serde::Serialize;

pub const PROBE_MEMBERS_NAME: &str = "__luar_probe";
pub const PROBE_SCOPE_NAME: &str = "__luar_scope";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    /// field / method / function / variable / constant / class / module
    pub kind: String,
    #[serde(rename = "type")]
    pub type_text: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletionReport {
    pub items: Vec<CompletionItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeKind {
    Members,
    Scope,
}

/// `__luar_probe(receiver)` の時点で分かったレシーバの型。定義ジャンプに使う。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReceiverInfo {
    /// クラスなら、そのクラスと祖先の名前 (子から親の順)。
    pub class_chain: Vec<String>,
    /// `Dog.` のようにクラスそのものか (インスタンスではない)。
    pub class_object: bool,
    /// 形の分かるテーブルか。
    pub is_shape: bool,
}

pub struct ProbeSource {
    pub source: String,
    pub kind: ProbeKind,
    /// メンバーアクセスのとき、レシーバ式の元の文字列。
    pub receiver: Option<String>,
    /// 入力途中で閉じ括弧が足りない場合に試す、閉じ括弧を補った版。
    pub repaired: Option<String>,
}

pub fn type_expr_text(ty: &TypeExpr) -> String {
    ty.to_string()
}

pub fn params_text(params: &[Param]) -> String {
    params
        .iter()
        .map(|param| match param {
            Param::Vararg => "...".to_string(),
            Param::Named { name, ty: None } => name.clone(),
            Param::Named { name, ty: Some(ty) } => format!("{name}: {}", type_expr_text(ty)),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn is_ident_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

/// UTF-16 のオフセットを `char` の位置へ変換する。範囲外は末尾へ丸める。
fn char_index(chars: &[char], utf16_offset: usize) -> usize {
    let mut units = 0;
    for (index, character) in chars.iter().enumerate() {
        if units >= utf16_offset {
            return index;
        }
        units += character.len_utf16();
    }
    chars.len()
}

/// `dot` (`.` または `:` の位置) の直前にあるレシーバ式の開始位置。
/// `a.b`, `a.b(1).c`, `a[1].b` のような識別子・呼び出し・添字の連鎖を対象にする。
fn receiver_start(chars: &[char], dot: usize) -> Option<usize> {
    let mut index = dot;
    loop {
        if index == 0 {
            break;
        }
        let previous = chars[index - 1];
        if previous == ')' || previous == ']' {
            let (open, close) = if previous == ')' { ('(', ')') } else { ('[', ']') };
            let mut depth = 0usize;
            let mut cursor = index;
            loop {
                if cursor == 0 {
                    return None;
                }
                cursor -= 1;
                if chars[cursor] == close {
                    depth += 1;
                } else if chars[cursor] == open {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            index = cursor;
            // 呼び出し・添字の前には式が続く必要がある。
            if index == 0 || !(is_ident_char(chars[index - 1]) || matches!(chars[index - 1], ')' | ']')) {
                return None;
            }
            continue;
        }
        if is_ident_char(previous) {
            while index > 0 && is_ident_char(chars[index - 1]) {
                index -= 1;
            }
            let joins = index > 0
                && (chars[index - 1] == '.'
                    && (index < 2 || chars[index - 2] != '.')
                    || chars[index - 1] == ':' && (index < 2 || chars[index - 2] != ':'));
            if joins {
                index -= 1;
                continue;
            }
            break;
        }
        break;
    }
    (index < dot).then_some(index)
}

/// 行内で閉じられていない括弧に対応する閉じ括弧。文字列は単純に読み飛ばす。
fn unclosed_closers(line: &str) -> String {
    let mut stack = Vec::new();
    let mut quote: Option<char> = None;
    for character in line.chars() {
        if let Some(open_quote) = quote {
            if character == open_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => quote = Some(character),
            '(' => stack.push(')'),
            '[' => stack.push(']'),
            '{' => stack.push('}'),
            ')' | ']' | '}' => {
                stack.pop();
            }
            _ => {}
        }
    }
    stack.iter().rev().collect()
}

fn probe_source(
    head: &str,
    probe: &str,
    tail: &str,
    kind: ProbeKind,
    receiver: Option<String>,
) -> ProbeSource {
    let line_start = head.rfind('\n').map_or(0, |index| index + 1);
    let closers = unclosed_closers(&head[line_start..]);
    ProbeSource {
        source: format!("{head}{probe}{tail}"),
        kind,
        receiver,
        repaired: (!closers.is_empty()).then(|| format!("{head}{probe}{closers}{tail}")),
    }
}

/// カーソル位置の式を、チェッカーが記録できる呼び出しへ書き換える。
pub fn build_probe(source: &str, utf16_offset: usize) -> ProbeSource {
    let chars: Vec<char> = source.chars().collect();
    let cursor = char_index(&chars, utf16_offset);
    let mut prefix_start = cursor;
    while prefix_start > 0 && is_ident_char(chars[prefix_start - 1]) {
        prefix_start -= 1;
    }
    let mut end = cursor;
    while end < chars.len() && is_ident_char(chars[end]) {
        end += 1;
    }

    if prefix_start > 0 {
        let separator = chars[prefix_start - 1];
        let is_access = (separator == '.' && (prefix_start < 2 || chars[prefix_start - 2] != '.'))
            || (separator == ':' && (prefix_start < 2 || chars[prefix_start - 2] != ':'));
        if is_access {
            if let Some(start) = receiver_start(&chars, prefix_start - 1) {
                let receiver: String = chars[start..prefix_start - 1].iter().collect();
                let head: String = chars[..start].iter().collect();
                let tail: String = chars[end..].iter().collect();
                let probe = format!("{PROBE_MEMBERS_NAME}({receiver})");
                return probe_source(&head, &probe, &tail, ProbeKind::Members, Some(receiver));
            }
        }
    }

    let head: String = chars[..prefix_start].iter().collect();
    let tail: String = chars[end..].iter().collect();
    let probe = format!("{PROBE_SCOPE_NAME}()");
    probe_source(&head, &probe, &tail, ProbeKind::Scope, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(source_with_cursor: &str) -> ProbeSource {
        let offset = source_with_cursor.find('|').unwrap();
        let source = source_with_cursor.replacen('|', "", 1);
        let utf16 = source[..offset].encode_utf16().count();
        build_probe(&source, utf16)
    }

    #[test]
    fn member_access_becomes_a_members_probe() {
        let result = probe("print(clsdef.dog.|)");
        assert_eq!(result.kind, ProbeKind::Members);
        assert_eq!(result.source, "print(__luar_probe(clsdef.dog))");
    }

    #[test]
    fn partial_member_name_is_dropped_including_the_rest_of_the_word() {
        let result = probe("local v = a.b(1).na|me\n");
        assert_eq!(result.kind, ProbeKind::Members);
        assert_eq!(result.source, "local v = __luar_probe(a.b(1))\n");
    }

    #[test]
    fn unclosed_parentheses_get_a_repaired_variant() {
        let result = probe("print(clsdef.dog.|");
        assert_eq!(result.source, "print(__luar_probe(clsdef.dog)");
        assert_eq!(result.repaired.as_deref(), Some("print(__luar_probe(clsdef.dog))"));
        let balanced = probe("print(clsdef.|)");
        assert!(balanced.repaired.is_some());
    }

    #[test]
    fn plain_identifier_becomes_a_scope_probe() {
        let result = probe("local x = 1\npr|\n");
        assert_eq!(result.kind, ProbeKind::Scope);
        assert_eq!(result.source, "local x = 1\n__luar_scope()\n");
    }

    #[test]
    fn concatenation_dots_are_not_member_access() {
        let result = probe("local s = a ..|");
        assert_eq!(result.kind, ProbeKind::Scope);
    }

    #[test]
    fn method_call_colon_is_member_access() {
        let result = probe("obj:|");
        assert_eq!(result.kind, ProbeKind::Members);
        assert_eq!(result.source, "__luar_probe(obj)");
    }
}
