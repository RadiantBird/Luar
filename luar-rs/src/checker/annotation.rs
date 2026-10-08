//! 型注釈の解決、`template` / `type` 宣言の検査、ジェネリックな関数呼び出しの検査。
//! `Checker` の一部で、型格子(`ValueType`)は親モジュールが持つ。

use super::{Checker, FnSig, ValueType};
use crate::ast::{Param, Program, Stmt, TypeExpr};
use crate::lexer::SourceSpan;
use std::collections::HashMap;

/// 型エイリアスの展開を辿る深さの上限。再帰する型(`type Node = { next: Node? }`)を
/// 有限の深さで打ち切り、以降は Unknown として扱う。
const MAX_ALIAS_DEPTH: usize = 6;

#[derive(Clone)]
pub(super) struct AliasInfo {
    pub params: Vec<String>,
    pub ty: TypeExpr,
    /// 宣言した `.luard` のモジュール名。自ファイルの型は None。
    pub module: Option<String>,
    pub is_export: bool,
}

/// 注釈を `ValueType` へ解決するときの文脈。
pub(super) struct TypeCtx<'a> {
    /// 見えている型引数名。
    pub params: &'a [String],
    /// 注釈を書いた `.luard` のモジュール名。その中ではexportされていない型も見える。
    pub module: Option<&'a str>,
    pub depth: usize,
}

/// 型名の検査の文脈。
struct Validation<'a> {
    params: &'a [String],
    module: Option<&'a str>,
    /// 未定義の型名をエラーにするか。新構文の宣言だけが true。
    strict: bool,
    line: usize,
    prefix: &'a str,
}

impl Checker {
    pub(super) fn type_from_annotation(&self, ty: &TypeExpr) -> ValueType {
        let ctx = TypeCtx {
            params: &self.scope_params,
            module: None,
            depth: 0,
        };
        self.resolve_annotation(ty, &ctx)
    }

    pub(super) fn resolve_annotation(&self, ty: &TypeExpr, ctx: &TypeCtx) -> ValueType {
        match ty {
            TypeExpr::Optional(inner) => {
                ValueType::Optional(Box::new(self.resolve_annotation(inner, ctx)))
            }
            TypeExpr::Tuple(_) => ValueType::Unknown,
            TypeExpr::Union(members) => ValueType::union_of(
                members
                    .iter()
                    .map(|member| self.resolve_annotation(member, ctx))
                    .collect(),
            ),
            // 要素の型は追跡せず、形の分からないテーブルとして扱う。
            TypeExpr::Array(_) => ValueType::Table,
            TypeExpr::Name(name) => self.resolve_named(name, &[], ty, ctx),
            TypeExpr::Generic { name, args } => self.resolve_named(name, args, ty, ctx),
            TypeExpr::Table(fields) => ValueType::Record {
                name: ty.to_string(),
                fields: fields
                    .iter()
                    .map(|(name, field)| (name.clone(), self.resolve_annotation(field, ctx)))
                    .collect(),
            },
            TypeExpr::Function { params, ret } => {
                let mut resolved = Vec::new();
                let mut vararg = None;
                for param in params {
                    match param {
                        TypeExpr::Vararg(element) => {
                            vararg = Some(self.resolve_annotation(element, ctx));
                        }
                        other => resolved.push(self.resolve_annotation(other, ctx)),
                    }
                }
                ValueType::FunctionSig(Box::new(FnSig {
                    type_params: Vec::new(),
                    params: resolved,
                    vararg,
                    ret: self.resolve_annotation(ret, ctx),
                    check_args: true,
                }))
            }
            TypeExpr::Vararg(element) => self.resolve_annotation(element, ctx),
        }
    }

    fn primitive_type(name: &str) -> Option<ValueType> {
        match name {
            "nil" => Some(ValueType::Nil),
            "boolean" => Some(ValueType::Boolean),
            "number" => Some(ValueType::Number),
            "string" => Some(ValueType::String),
            "table" => Some(ValueType::Table),
            "function" => Some(ValueType::Function),
            "any" | "unknown" => Some(ValueType::Unknown),
            _ => None,
        }
    }

    fn resolve_named(
        &self,
        name: &str,
        args: &[TypeExpr],
        ty: &TypeExpr,
        ctx: &TypeCtx,
    ) -> ValueType {
        if args.is_empty() {
            if let Some(primitive) = Self::primitive_type(name) {
                return primitive;
            }
            if ctx.params.iter().any(|param| param == name) {
                return ValueType::Generic(name.to_string());
            }
        }
        let resolved: Vec<ValueType> = args
            .iter()
            .map(|arg| self.resolve_annotation(arg, ctx))
            .collect();
        if let Some(alias) = self.find_alias(name, ctx.module) {
            return self.expand_alias(alias, ty.to_string(), resolved, ctx);
        }
        if self.classes.contains_key(name) {
            return ValueType::Class(name.to_string(), resolved);
        }
        ValueType::Unknown
    }

    /// 自ファイルの型、`mod.Name`(export済み)、`.luard` 内の非exportの型の順に探す。
    fn find_alias(&self, name: &str, module: Option<&str>) -> Option<&AliasInfo> {
        if let Some(module) = module {
            if let Some(alias) = self.aliases.get(&format!("{module}.{name}")) {
                return Some(alias);
            }
        }
        self.aliases
            .get(name)
            .filter(|alias| alias.module.is_none() || alias.is_export)
    }

    fn expand_alias(
        &self,
        alias: &AliasInfo,
        display: String,
        mut args: Vec<ValueType>,
        ctx: &TypeCtx,
    ) -> ValueType {
        if ctx.depth >= MAX_ALIAS_DEPTH {
            return ValueType::Unknown;
        }
        args.resize(alias.params.len(), ValueType::Unknown);
        let inner = TypeCtx {
            params: &alias.params,
            module: alias.module.as_deref(),
            depth: ctx.depth + 1,
        };
        let body = self.resolve_annotation(&alias.ty, &inner);
        let bindings: HashMap<String, ValueType> =
            alias.params.iter().cloned().zip(args).collect();
        match Self::substitute(&body, &bindings) {
            ValueType::Record { fields, .. } => ValueType::Record {
                name: display,
                fields,
            },
            other => other,
        }
    }

    /// 型引数を実際の型へ置き換える。
    pub(super) fn substitute(ty: &ValueType, bindings: &HashMap<String, ValueType>) -> ValueType {
        let each = |types: &[ValueType], bindings: &HashMap<String, ValueType>| {
            types
                .iter()
                .map(|ty| Self::substitute(ty, bindings))
                .collect::<Vec<_>>()
        };
        let fields = |fields: &[(String, ValueType)]| {
            fields
                .iter()
                .map(|(name, ty)| (name.clone(), Self::substitute(ty, bindings)))
                .collect::<Vec<_>>()
        };
        match ty {
            ValueType::Generic(name) => bindings.get(name).cloned().unwrap_or_else(|| ty.clone()),
            ValueType::Optional(inner) => {
                ValueType::union_of(vec![
                    Self::substitute(inner, bindings),
                    ValueType::Nil,
                ])
            }
            ValueType::Union(members) => ValueType::union_of(each(members, bindings)),
            ValueType::Shape(items) => ValueType::Shape(fields(items)),
            ValueType::Record { name, fields: items } => ValueType::Record {
                name: name.clone(),
                fields: fields(items),
            },
            ValueType::Class(name, args) => ValueType::Class(name.clone(), each(args, bindings)),
            ValueType::FunctionSig(sig) => {
                // 関数自身の型引数は外側の束縛で隠される。
                let mut inner = bindings.clone();
                for param in &sig.type_params {
                    inner.remove(param);
                }
                ValueType::FunctionSig(Box::new(FnSig {
                    type_params: sig.type_params.clone(),
                    params: each(&sig.params, &inner),
                    vararg: sig.vararg.as_ref().map(|ty| Self::substitute(ty, &inner)),
                    ret: Self::substitute(&sig.ret, &inner),
                    check_args: sig.check_args,
                }))
            }
            _ => ty.clone(),
        }
    }

    // ─── 型宣言の登録と検査 ─────────────────────────────────────────────────

    /// 自ファイルの `type` と、`import type` したmoduleの `type` を登録する。
    pub(super) fn register_aliases(&mut self, program: &Program) {
        self.aliases.clear();
        let module_aliases: Vec<(String, AliasInfo)> = self
            .modules
            .iter()
            .flat_map(|module| {
                module.types.iter().map(|declared| {
                    (
                        format!("{}.{}", module.name, declared.name),
                        AliasInfo {
                            params: declared.type_params.clone(),
                            ty: declared.ty.clone(),
                            module: Some(module.name.clone()),
                            is_export: declared.is_export,
                        },
                    )
                })
            })
            .collect();
        self.aliases.extend(module_aliases);
        for stmt in &program.stmts {
            let Stmt::TypeAlias {
                is_export,
                name,
                type_params,
                ty,
                line,
            } = stmt
            else {
                continue;
            };
            if self.aliases.contains_key(name) {
                self.err(format!("type '{name}' is already defined"), *line);
                continue;
            }
            self.aliases.insert(
                name.clone(),
                AliasInfo {
                    params: type_params.clone(),
                    ty: ty.clone(),
                    module: None,
                    is_export: *is_export,
                },
            );
        }
    }

    /// 自ファイルの型宣言と、`declare function` の署名を検査する。
    pub(super) fn validate_declarations(&mut self, program: &Program) {
        for stmt in &program.stmts {
            match stmt {
                Stmt::TypeAlias {
                    name,
                    type_params,
                    ty,
                    line,
                    ..
                } => {
                    let prefix = format!("in type '{name}': ");
                    self.validate_type_with(ty, type_params, None, true, *line, &prefix);
                }
                Stmt::DeclareFunction {
                    name,
                    type_params,
                    params,
                    return_type,
                    line,
                    ..
                } => {
                    let prefix = format!("in 'declare function {name}': ");
                    self.validate_signature(
                        type_params,
                        params,
                        return_type.as_ref(),
                        None,
                        true,
                        *line,
                        &prefix,
                    );
                }
                _ => {}
            }
        }
    }

    /// `import type` したmoduleの型宣言と `declare function` を検査する。
    pub(super) fn validate_module_declarations(&mut self) {
        let modules = self.modules.clone();
        for module in &modules {
            let origin = format!("{}.luard", module.name);
            for declared in &module.types {
                let prefix = format!("{origin}: in type '{}': ", declared.name);
                self.validate_type_with(
                    &declared.ty,
                    &declared.type_params,
                    Some(&module.name),
                    true,
                    1,
                    &prefix,
                );
            }
            for function in &module.functions {
                let prefix = format!("{origin}: in 'declare function {}': ", function.name);
                self.validate_signature(
                    &function.type_params,
                    &function.params,
                    function.return_type.as_ref(),
                    Some(&module.name),
                    true,
                    1,
                    &prefix,
                );
            }
        }
    }

    /// 関数の署名の型名を検査する。`strict` は未定義の型名もエラーにする。
    pub(super) fn validate_signature(
        &mut self,
        type_params: &[String],
        params: &[Param],
        return_type: Option<&TypeExpr>,
        module: Option<&str>,
        strict: bool,
        line: usize,
        prefix: &str,
    ) {
        let mut visible = self.scope_params.clone();
        visible.extend(type_params.iter().cloned());
        for param in params {
            if let Param::Named { ty: Some(ty), .. } = param {
                self.validate_type_with(ty, &visible, module, strict, line, prefix);
            }
        }
        if let Some(ty) = return_type {
            self.validate_type_with(ty, &visible, module, strict, line, prefix);
        }
    }

    /// 型注釈1つの型名と型引数の個数を検査する。
    pub(super) fn validate_type_with(
        &mut self,
        ty: &TypeExpr,
        params: &[String],
        module: Option<&str>,
        strict: bool,
        line: usize,
        prefix: &str,
    ) {
        let validation = Validation {
            params,
            module,
            strict,
            line,
            prefix,
        };
        self.validate_type(ty, &validation);
    }

    fn validate_type(&mut self, ty: &TypeExpr, validation: &Validation) {
        match ty {
            TypeExpr::Name(name) => self.validate_name(name, 0, validation),
            TypeExpr::Generic { name, args } => {
                self.validate_name(name, args.len(), validation);
                for arg in args {
                    self.validate_type(arg, validation);
                }
            }
            TypeExpr::Optional(inner) | TypeExpr::Array(inner) | TypeExpr::Vararg(inner) => {
                self.validate_type(inner, validation)
            }
            TypeExpr::Tuple(types) | TypeExpr::Union(types) => {
                for ty in types {
                    self.validate_type(ty, validation);
                }
            }
            TypeExpr::Table(fields) => {
                for (_, ty) in fields {
                    self.validate_type(ty, validation);
                }
            }
            TypeExpr::Function { params, ret } => {
                for ty in params {
                    self.validate_type(ty, validation);
                }
                self.validate_type(ret, validation);
            }
        }
    }

    fn validate_name(&mut self, name: &str, arg_count: usize, validation: &Validation) {
        let is_param = validation.params.iter().any(|param| param == name);
        if Self::primitive_type(name).is_some() || is_param {
            if arg_count > 0 {
                self.report_type_error(
                    format!("type '{name}' does not take type arguments"),
                    validation,
                );
            }
            return;
        }
        let expected = match self.find_alias(name, validation.module) {
            Some(alias) => Some(alias.params.len()),
            // クラス名だけの注釈 (`static function new(): Box`) は型引数を省略した形として許す。
            None => self
                .classes
                .get(name)
                .map(|class| if arg_count == 0 { 0 } else { class.type_params.len() }),
        };
        match expected {
            Some(count) if count == arg_count => {}
            Some(count) => self.report_type_error(
                format!("type '{name}' expects {count} type argument(s), got {arg_count}"),
                validation,
            ),
            None if validation.strict || arg_count > 0 => {
                self.report_type_error(format!("unknown type '{name}'"), validation)
            }
            None => {}
        }
    }

    fn report_type_error(&mut self, message: String, validation: &Validation) {
        self.err(format!("{}{message}", validation.prefix), validation.line);
    }

    // ─── 関数の署名と呼び出し ───────────────────────────────────────────────

    /// 宣言から関数の型を作る。`type_params` はその関数自身の型引数。
    pub(super) fn build_sig(
        &self,
        type_params: &[String],
        params: &[Param],
        return_type: Option<&TypeExpr>,
        module: Option<&str>,
    ) -> FnSig {
        let mut visible = self.scope_params.clone();
        visible.extend(type_params.iter().cloned());
        let ctx = TypeCtx {
            params: &visible,
            module,
            depth: 0,
        };
        let mut sig = FnSig {
            type_params: type_params.to_vec(),
            params: Vec::new(),
            vararg: None,
            ret: return_type
                .map(|ty| self.resolve_annotation(ty, &ctx))
                .unwrap_or(ValueType::Unknown),
            check_args: true,
        };
        for param in params {
            match param {
                Param::Vararg => sig.vararg = Some(ValueType::Unknown),
                Param::Named { ty, .. } => sig.params.push(
                    ty.as_ref()
                        .map(|ty| self.resolve_annotation(ty, &ctx))
                        .unwrap_or(ValueType::Unknown),
                ),
            }
        }
        sig
    }

    /// 呼び出しの引数を型引数ごとに突き合わせ、戻り値の型を返す。
    pub(super) fn check_call(
        &mut self,
        sig: &FnSig,
        callee_name: &str,
        callee: &crate::ast::Expr,
        args: &[crate::ast::Expr],
        arg_types: &[ValueType],
    ) -> ValueType {
        if !sig.check_args {
            return sig.ret.clone();
        }
        let callee_span = Self::expr_span(callee);
        let required = sig
            .params
            .iter()
            .rposition(|param| !Self::is_omittable(param))
            .map_or(0, |index| index + 1);
        if (sig.vararg.is_none() && args.len() > sig.params.len()) || args.len() < required {
            let maximum = sig.params.len();
            let expected = if sig.vararg.is_none() && required == maximum {
                maximum.to_string()
            } else if args.len() < required {
                format!("at least {required}")
            } else {
                format!("at most {maximum}")
            };
            self.report_call_error(
                format!(
                    "function '{callee_name}' expects {expected} argument(s), got {}",
                    args.len()
                ),
                callee_span,
            );
        }
        let mut bindings = HashMap::new();
        for (index, actual) in arg_types.iter().enumerate() {
            let Some(param) = sig.params.get(index).or(sig.vararg.as_ref()) else {
                break;
            };
            let result = Self::unify(
                param,
                actual,
                &sig.type_params,
                &mut bindings,
                index + 1,
                callee_name,
            );
            if let Err(message) = result {
                let span = args.get(index).and_then(Self::expr_span).or(callee_span);
                self.report_call_error(message, span);
            }
        }
        // 引数から決まらなかった型引数は Unknown にする。
        let mut result_bindings: HashMap<String, ValueType> = sig
            .type_params
            .iter()
            .map(|name| (name.clone(), ValueType::Unknown))
            .collect();
        result_bindings.extend(bindings);
        Self::substitute(&sig.ret, &result_bindings)
    }

    fn is_omittable(param: &ValueType) -> bool {
        matches!(param, ValueType::Optional(_) | ValueType::Unknown)
    }

    fn report_call_error(&mut self, message: String, span: Option<SourceSpan>) {
        match span {
            Some(span) => self.err_at(message, span),
            None => self.err(message, 1),
        }
    }

    /// 式の先頭の識別子の位置。呼び出し式自体は位置を持たないため、これで代用する。
    fn expr_span(expr: &crate::ast::Expr) -> Option<SourceSpan> {
        use crate::ast::Expr;
        match expr {
            Expr::Ident { span, .. } => Some(*span),
            Expr::Field { obj, .. } | Expr::Index { obj, .. } => Self::expr_span(obj),
            Expr::MethodCall { obj, .. } => Self::expr_span(obj),
            Expr::Call { callee, .. } => Self::expr_span(callee),
            Expr::Binop { span, .. } => Some(*span),
            _ => None,
        }
    }

    fn unify(
        param: &ValueType,
        actual: &ValueType,
        type_params: &[String],
        bindings: &mut HashMap<String, ValueType>,
        position: usize,
        callee_name: &str,
    ) -> Result<(), String> {
        match param {
            ValueType::Generic(name) if type_params.contains(name) => {
                if *actual == ValueType::Unknown {
                    return Ok(());
                }
                match bindings.get(name) {
                    None => {
                        bindings.insert(name.clone(), actual.clone());
                        Ok(())
                    }
                    Some(bound) if Self::is_assignable(bound, actual) => Ok(()),
                    Some(bound) => Err(format!(
                        "type parameter '{name}' was inferred as {} but argument {position} is {}",
                        bound.display(),
                        actual.display()
                    )),
                }
            }
            ValueType::Optional(inner) => match actual {
                ValueType::Nil => Ok(()),
                ValueType::Optional(actual_inner) => Self::unify(
                    inner,
                    actual_inner,
                    type_params,
                    bindings,
                    position,
                    callee_name,
                ),
                other => Self::unify(inner, other, type_params, bindings, position, callee_name),
            },
            ValueType::Class(name, args) if !args.is_empty() => {
                if let ValueType::Class(actual_name, actual_args) = actual {
                    if name == actual_name {
                        for (param_arg, actual_arg) in args.iter().zip(actual_args) {
                            Self::unify(
                                param_arg,
                                actual_arg,
                                type_params,
                                bindings,
                                position,
                                callee_name,
                            )?;
                        }
                        return Ok(());
                    }
                }
                Self::plain_argument(param, actual, bindings, position, callee_name)
            }
            ValueType::Record { fields, .. } => {
                if let ValueType::Shape(actual_fields) | ValueType::Record { fields: actual_fields, .. } =
                    actual
                {
                    for (field, field_type) in fields {
                        if let Some((_, actual_field)) =
                            actual_fields.iter().find(|(name, _)| name == field)
                        {
                            Self::unify(
                                field_type,
                                actual_field,
                                type_params,
                                bindings,
                                position,
                                callee_name,
                            )?;
                        }
                    }
                    return Ok(());
                }
                Self::plain_argument(param, actual, bindings, position, callee_name)
            }
            _ => Self::plain_argument(param, actual, bindings, position, callee_name),
        }
    }

    /// 型引数を含まない引数の代入可否。束縛済みの型引数は置き換えてから比べる。
    fn plain_argument(
        param: &ValueType,
        actual: &ValueType,
        bindings: &HashMap<String, ValueType>,
        position: usize,
        callee_name: &str,
    ) -> Result<(), String> {
        let expected = Self::substitute(param, bindings);
        if Self::is_assignable(&expected, actual) {
            Ok(())
        } else {
            Err(format!(
                "argument {position} of '{callee_name}' expects {}, got {}",
                expected.display(),
                actual.display()
            ))
        }
    }

    /// 構造型 `expected` に `actual` のフィールドが足りない、または型が合わない最初の理由。
    pub(super) fn record_mismatch(
        expected: &[(String, ValueType)],
        actual: &[(String, ValueType)],
    ) -> Option<String> {
        for (name, expected_type) in expected {
            match actual.iter().find(|(field, _)| field == name) {
                Some((_, actual_type)) => {
                    if !Self::is_assignable(expected_type, actual_type) {
                        return Some(format!(
                            "field '{name}' expects {}, got {}",
                            expected_type.display(),
                            actual_type.display()
                        ));
                    }
                }
                None if !Self::is_omittable(expected_type) => {
                    return Some(format!("missing field '{name}'"));
                }
                None => {}
            }
        }
        None
    }

    /// 代入できない理由が構造型のフィールドにあるとき、その説明。
    pub(super) fn mismatch_detail(expected: &ValueType, actual: &ValueType) -> String {
        match (expected, actual) {
            (
                ValueType::Record { fields: expected, .. },
                ValueType::Shape(actual) | ValueType::Record { fields: actual, .. },
            ) => Self::record_mismatch(expected, actual)
                .map(|reason| format!(" ({reason})"))
                .unwrap_or_default(),
            _ => String::new(),
        }
    }
}

// ─── 型キャスト ─────────────────────────────────────────────────────────────

impl Checker {
    /// `from :: to` を許すか。元と先が関連する型(一方が他方へ代入できる、継承関係にある、
    /// `T?` から `T` への絞り込み、Unknown など)のときだけ許す。
    pub(super) fn cast_allowed(&self, from: &ValueType, to: &ValueType) -> bool {
        // ユニオンは、どれか1つのメンバーが関連していれば許す。
        if let ValueType::Union(members) = from {
            return members.iter().any(|member| self.cast_allowed(member, to));
        }
        if let ValueType::Union(members) = to {
            return members.iter().any(|member| self.cast_allowed(from, member));
        }
        if from.is_opaque() || to.is_opaque() {
            return true;
        }
        if Self::is_assignable(to, from) || Self::is_assignable(from, to) {
            return true;
        }
        match (from, to) {
            (ValueType::Optional(inner), other) | (other, ValueType::Optional(inner)) => {
                self.cast_allowed(inner, other)
            }
            (ValueType::Class(a, _), ValueType::Class(b, _)) => {
                self.is_subclass(a, b) || self.is_subclass(b, a)
            }
            // `setmetatable({}, Dog) :: Dog` のように、テーブルをクラスとして扱う。
            (ValueType::Table | ValueType::Shape(_) | ValueType::Record { .. }, ValueType::Class(..)) => {
                true
            }
            _ => false,
        }
    }

    /// `child` が `ancestor` と同じか、その子孫か。
    fn is_subclass(&self, child: &str, ancestor: &str) -> bool {
        let mut current = Some(child.to_string());
        let mut seen = std::collections::HashSet::new();
        while let Some(name) = current {
            if name == ancestor {
                return true;
            }
            if !seen.insert(name.clone()) {
                return false;
            }
            current = self
                .classes
                .get(&name)
                .and_then(|class| class.parent_name.clone());
        }
        false
    }
}
