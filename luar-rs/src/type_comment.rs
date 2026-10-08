//! 生成コードに書き出す、型注釈の元のシグネチャを表すコメントの文字列を作る。
//! 型は出力コードでは消去されるため、コンパイラの不具合調査用に元の宣言を残す。

use crate::ast::{ClassDecl, Member, MethodMember, Param, TypeExpr};

fn type_params_text(type_params: &[String]) -> String {
    if type_params.is_empty() {
        String::new()
    } else {
        format!("<{}>", type_params.join(", "))
    }
}

fn param_text(param: &Param) -> String {
    match param {
        Param::Vararg => "...".to_string(),
        Param::Named { name, ty: None } => name.clone(),
        Param::Named { name, ty: Some(ty) } => format!("{name}: {ty}"),
    }
}

fn params_text(params: &[Param]) -> String {
    params.iter().map(param_text).collect::<Vec<_>>().join(", ")
}

fn has_typed_param(params: &[Param]) -> bool {
    params
        .iter()
        .any(|param| matches!(param, Param::Named { ty: Some(_), .. }))
}

/// `-- cast: tonumber(a) :: number`。`indent` は文のインデント。
pub fn cast_comment(indent: &str, cast: &str) -> String {
    format!("{indent}-- cast: {cast}")
}

/// `-- export type MyTable<T> = { id: number, ref: T }`
pub fn type_alias_comment(
    is_export: bool,
    name: &str,
    type_params: &[String],
    ty: &TypeExpr,
) -> String {
    let export = if is_export { "export " } else { "" };
    format!(
        "-- {export}type {name}{} = {ty}",
        type_params_text(type_params)
    )
}

/// `-- local a: number, b`。型注釈が1つも無ければ `None`。
pub fn binding_comment(
    keyword: &str,
    names: &[String],
    types: &[Option<TypeExpr>],
) -> Option<String> {
    if types.iter().all(Option::is_none) {
        return None;
    }
    let bindings = names
        .iter()
        .enumerate()
        .map(|(index, name)| match types.get(index).and_then(Option::as_ref) {
            Some(ty) => format!("{name}: {ty}"),
            None => name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("-- {keyword} {bindings}"))
}

/// `-- function add<T>(a: T, b: string): number`。型に関わる情報が無ければ `None`。
pub fn function_comment(
    prefix: &str,
    name: &str,
    type_params: &[String],
    params: &[Param],
    return_type: Option<&TypeExpr>,
) -> Option<String> {
    if type_params.is_empty() && !has_typed_param(params) && return_type.is_none() {
        return None;
    }
    let ret = return_type.map(|ty| format!(": {ty}")).unwrap_or_default();
    Some(format!(
        "-- {prefix}function {name}{}({}){ret}",
        type_params_text(type_params),
        params_text(params)
    ))
}

fn method_comment(method: &MethodMember) -> Option<String> {
    let prefix = if method.is_static { "static " } else { "" };
    function_comment(
        prefix,
        &method.name,
        &method.type_params,
        &method.params,
        method.return_type.as_ref(),
    )
}

/// クラスの型情報(型引数、型つきフィールド、型つきメソッド)をコメント行の列にする。
/// 型に関わる情報が無ければ空。
pub fn class_comments(decl: &ClassDecl) -> Vec<String> {
    let mut members = Vec::new();
    let all_members = decl
        .top_level_members
        .iter()
        .chain(decl.blocks.iter().flat_map(|block| block.members.iter()));
    for member in all_members {
        match member {
            Member::Field(field) => {
                if let Some(ty) = &field.ty {
                    members.push(format!("--   {}: {ty}", field.name));
                }
            }
            Member::Method(method) => {
                if let Some(comment) = method_comment(method) {
                    members.push(format!("--  {}", &comment[2..]));
                }
            }
        }
    }
    if decl.type_params.is_empty() && members.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "-- class {}{}",
        decl.name,
        type_params_text(&decl.type_params)
    )];
    lines.extend(members);
    lines
}
