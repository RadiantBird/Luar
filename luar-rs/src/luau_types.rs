//! Luau ターゲットの出力コードへ書く型注釈の文字列を作る。
//! Luau の型構文は Luar とほぼ同じなので、型の名前だけを解決し直す。
//! 出力コードの中で宣言されていない名前(クラス、import した型など)は、
//! Luau 側で未定義の型エラーにならないよう `any` へ落とす。

use crate::ast::TypeExpr;

const BUILTIN_TYPES: &[&str] = &[
    "number", "string", "boolean", "nil", "any", "unknown", "never", "table", "function", "thread",
    "buffer", "vector",
];

pub fn is_builtin(name: &str) -> bool {
    BUILTIN_TYPES.contains(&name)
}

/// `known` が真を返す名前だけをそのまま出し、それ以外の名前は `any` にする。
pub fn render(ty: &TypeExpr, known: &dyn Fn(&str) -> bool) -> String {
    match ty {
        TypeExpr::Name(name) => {
            if is_builtin(name) || known(name) {
                name.clone()
            } else {
                "any".to_string()
            }
        }
        TypeExpr::Generic { name, args } => {
            if known(name) {
                format!("{name}<{}>", render_list(args, known))
            } else {
                "any".to_string()
            }
        }
        TypeExpr::Optional(inner) => match inner.as_ref() {
            TypeExpr::Union(_) | TypeExpr::Function { .. } => {
                format!("({})?", render(inner, known))
            }
            _ => format!("{}?", render(inner, known)),
        },
        TypeExpr::Union(members) => members
            .iter()
            .map(|member| match member {
                TypeExpr::Function { .. } => format!("({})", render(member, known)),
                _ => render(member, known),
            })
            .collect::<Vec<_>>()
            .join(" | "),
        TypeExpr::Tuple(types) => format!("({})", render_list(types, known)),
        TypeExpr::Table(fields) => {
            let fields = fields
                .iter()
                .map(|(name, ty)| format!("{name}: {}", render(ty, known)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{ {fields} }}")
        }
        TypeExpr::Array(element) => format!("{{ {} }}", render(element, known)),
        TypeExpr::Vararg(element) => format!("...{}", render(element, known)),
        TypeExpr::Function { params, ret } => {
            format!("({}) -> {}", render_list(params, known), render(ret, known))
        }
    }
}

fn render_list(types: &[TypeExpr], known: &dyn Fn(&str) -> bool) -> String {
    types
        .iter()
        .map(|ty| render(ty, known))
        .collect::<Vec<_>>()
        .join(", ")
}
