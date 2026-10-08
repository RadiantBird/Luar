use crate::ast::*;
use crate::completion::{
    CompletionItem, PROBE_MEMBERS_NAME, PROBE_SCOPE_NAME, ReceiverInfo, params_text,
    type_expr_text,
};
use crate::lexer::SourceSpan;
use crate::modules::{DeclaredClass, DeclaredFunction, ModuleDefinition};
use std::collections::{HashMap, HashSet};

mod annotation;

use annotation::AliasInfo;

#[derive(Debug, Clone)]
pub struct CheckError {
    pub message: String,
    pub line: usize,
    pub span: Option<SourceSpan>,
}

#[derive(Clone)]
struct ClassInfo {
    name: String,
    type_params: Vec<String>,
    is_abstract: bool,
    parent_name: Option<String>,
    methods: HashMap<String, MethodInfo>,
    fields: HashMap<String, FieldInfo>,
    /// このクラスのprivateメンバーへのアクセスを許可したクラス。継承されない。
    friends: Vec<String>,
}

#[derive(Clone)]
struct MethodInfo {
    method: MethodMember,
    access: Access,
    class_name: String,
}

#[derive(Clone)]
struct FieldInfo {
    field: FieldMember,
    access: Access,
    class_name: String,
}

#[derive(Clone)]
enum ReceiverKind {
    Class(String),
    Instance(String),
}

/// コンパイル時に確定できる範囲だけを表す、意図的に小さな型格子。
/// Unknown は外部ランタイム値であり、推測だけで拒否しない。
#[derive(Clone, Debug, PartialEq, Eq)]
enum ValueType {
    Unknown,
    Nil,
    Boolean,
    Number,
    String,
    Table,
    /// フィールド名と型が分かっているテーブル。`{ dog = Dog.new() }` のような
    /// リテラルや、`mod.x = 1` による追加で形が決まる。順序は宣言順。
    Shape(Vec<(String, ValueType)>),
    Function,
    /// 引数型・戻り値型が分かっている関数 (`declare function` や `template` つきの関数)。
    FunctionSig(Box<FnSig>),
    /// `template <T>` で宣言された型引数。宣言の本体の中だけに現れる。
    Generic(String),
    /// 注釈由来の構造的なテーブル型 (`MyTable<number>` や `{ id: number }`)。
    /// `name` は表示用で、フィールドは宣言順。
    Record {
        name: String,
        fields: Vec<(String, ValueType)>,
    },
    /// クラスと、そのクラスの型引数 (`Box<number>`)。型引数が分からなければ空。
    Class(String, Vec<ValueType>),
    Optional(Box<ValueType>),
    /// ユニオン型 `A | B`。2つ以上のメンバーを持ち、`nil` は含まない
    /// (`nil` を含むものは `Optional` にする)。`ValueType::union_of` で作る。
    Union(Vec<ValueType>),
}

/// 関数の署名。`type_params` はこの関数自身の型引数。
#[derive(Clone, Debug, PartialEq, Eq)]
struct FnSig {
    type_params: Vec<String>,
    params: Vec<ValueType>,
    /// 可変長引数 `...T` の要素型。
    vararg: Option<ValueType>,
    ret: ValueType,
    /// 呼び出しの引数を検査するか。`template` / `declare` / 標準関数は true。
    /// 戻り値だけを推論した通常の関数は false で、従来どおり引数を検査しない。
    check_args: bool,
}

impl ValueType {
    fn is_function(&self) -> bool {
        matches!(self, Self::Function | Self::FunctionSig(_))
    }

    /// 演算子の検査で、型を決めつけない値 (外部の値と宣言本体の型引数)。
    fn is_opaque(&self) -> bool {
        matches!(self, Self::Unknown | Self::Generic(_))
    }

    /// 演算子の検査で拒否しない値。ユニオンは、どのメンバーかを追跡しないので許す。
    fn is_lenient_operand(&self) -> bool {
        self.is_opaque() || matches!(self, Self::Union(_))
    }

    /// ユニオンの正規形を作る。入れ子を平らにし、重複を除き、`nil` を含むなら `Optional` にする。
    /// メンバーに Unknown があれば全体が Unknown。
    fn union_of(members: Vec<ValueType>) -> ValueType {
        let mut flat: Vec<ValueType> = Vec::new();
        let mut has_nil = false;
        let mut pending = members;
        pending.reverse();
        while let Some(member) = pending.pop() {
            match member {
                Self::Unknown => return Self::Unknown,
                Self::Nil => has_nil = true,
                Self::Optional(inner) => {
                    has_nil = true;
                    pending.push(*inner);
                }
                Self::Union(inner) => pending.extend(inner.into_iter().rev()),
                other => {
                    if !flat.contains(&other) {
                        flat.push(other);
                    }
                }
            }
        }
        let base = match flat.len() {
            0 => return Self::Nil,
            1 => flat.remove(0),
            _ => Self::Union(flat),
        };
        if has_nil {
            Self::Optional(Box::new(base))
        } else {
            base
        }
    }

    fn display(&self) -> String {
        match self {
            Self::Unknown => "unknown".to_string(),
            Self::Nil => "nil".to_string(),
            Self::Boolean => "boolean".to_string(),
            Self::Number => "number".to_string(),
            Self::String => "string".to_string(),
            Self::Table | Self::Shape(_) => "table".to_string(),
            Self::Function => "function".to_string(),
            Self::FunctionSig(sig) => {
                let mut params: Vec<String> = sig.params.iter().map(Self::display).collect();
                if let Some(vararg) = &sig.vararg {
                    params.push(format!("...{}", vararg.display()));
                }
                format!("({}) -> {}", params.join(", "), sig.ret.display())
            }
            Self::Generic(name) => name.clone(),
            Self::Record { name, .. } => name.clone(),
            Self::Class(name, args) if args.is_empty() => name.clone(),
            Self::Class(name, args) => format!(
                "{name}<{}>",
                args.iter().map(Self::display).collect::<Vec<_>>().join(", ")
            ),
            Self::Optional(inner) if matches!(**inner, Self::Union(_)) => {
                format!("({})?", inner.display())
            }
            Self::Optional(inner) => format!("{}?", inner.display()),
            Self::Union(members) => members
                .iter()
                .map(Self::display)
                .collect::<Vec<_>>()
                .join(" | "),
        }
    }
}

pub struct Checker {
    errors: Vec<CheckError>,
    classes: HashMap<String, ClassInfo>,
    /// import type された .luard の宣言。型の初期値と declare class の供給元。
    modules: Vec<ModuleDefinition>,
    /// 補完用の `__luar_probe` / `__luar_scope` が記録した候補。
    probe: Option<Vec<CompletionItem>>,
    /// `__luar_probe` のレシーバの型。
    probe_receiver: Option<ReceiverInfo>,
    /// `const` で宣言された名前。補完で種別を示すためだけに使う近似。
    const_names: HashSet<String>,
    /// `type` 宣言。自ファイルは `Name`、`import type` したmoduleは `mod.Name` で引く。
    aliases: HashMap<String, AliasInfo>,
    /// 検査中の `template` 関数の型引数。注釈の解決で `T` を型引数として扱う。
    scope_params: Vec<String>,
    /// 標準ライブラリの宣言。最初の型環境に入る。
    builtins: Option<ModuleDefinition>,
    /// 検査中の関数ごとに、`return` した値の型を集める。戻り値の推論に使う。
    return_types: Vec<Vec<ValueType>>,
    /// インスタンスメソッドの呼び出し `obj.method(...)` のノードのアドレス。
    /// 検査の後で `obj:method(...)` に書き換える(`method_calls`)。
    instance_calls: HashSet<usize>,
    /// 書き換え対象の呼び出しを記録中か(本体の型検査のときだけ)。
    recording_calls: bool,
}

impl Checker {
    pub fn new() -> Self {
        Checker {
            errors: Vec::new(),
            classes: HashMap::new(),
            modules: Vec::new(),
            probe: None,
            probe_receiver: None,
            const_names: HashSet::new(),
            aliases: HashMap::new(),
            scope_params: Vec::new(),
            builtins: None,
            return_types: Vec::new(),
            instance_calls: HashSet::new(),
            recording_calls: false,
        }
    }

    /// 標準ライブラリ(`stdlib::definition`)を型環境の初期値にする。
    pub fn with_builtins(mut self, builtins: &ModuleDefinition) -> Self {
        self.builtins = Some(builtins.clone());
        self
    }

    pub fn with_modules(mut self, modules: Vec<ModuleDefinition>) -> Self {
        self.modules = modules;
        self
    }

    /// `check` の最中に補完用の呼び出しへ到達していれば、その候補。
    pub fn take_completions(&mut self) -> Option<Vec<CompletionItem>> {
        self.probe.take()
    }

    pub fn take_receiver(&mut self) -> Option<ReceiverInfo> {
        self.probe_receiver.take()
    }

    fn receiver_info(&self, ty: &ValueType, class_object: bool) -> ReceiverInfo {
        let mut info = ReceiverInfo {
            class_object,
            is_shape: matches!(ty, ValueType::Shape(_) | ValueType::Record { .. }),
            ..ReceiverInfo::default()
        };
        if let ValueType::Class(name, _) = ty {
            let mut seen = HashSet::new();
            let mut current = Some(name.clone());
            while let Some(class_name) = current {
                if !seen.insert(class_name.clone()) {
                    break;
                }
                current = self
                    .classes
                    .get(&class_name)
                    .and_then(|info| info.parent_name.clone());
                info.class_chain.push(class_name);
            }
        }
        info
    }

    pub fn check(&mut self, program: &mut Program) -> Vec<CheckError> {
        self.errors.clear();
        self.classes.clear();
        self.probe = None;
        self.probe_receiver = None;
        self.const_names.clear();
        // Pass 0: .luard の declare class を実体なしのクラスとして登録する
        let declared: Vec<DeclaredClass> = self
            .modules
            .iter()
            .flat_map(|module| module.classes.iter().cloned())
            .collect();
        for class in &declared {
            self.register_declared_class(class);
        }
        // Pass 1: register classes
        for stmt in &program.stmts {
            if let Stmt::ClassDecl(decl) = stmt {
                self.register_class(decl);
            }
        }

        // Pass 1.5: type 宣言を登録し、型宣言と declare function の署名を検査する
        self.scope_params.clear();
        self.register_aliases(program);
        self.validate_declarations(program);
        self.validate_module_declarations();

        // Pass 2: validate classes
        let decls: Vec<ClassDecl> = program
            .stmts
            .iter()
            .filter_map(|s| {
                if let Stmt::ClassDecl(d) = s {
                    Some(d.clone())
                } else {
                    None
                }
            })
            .collect();
        for decl in &decls {
            self.check_class(decl);
        }

        // Pass 3: access control
        self.check_access_control(program);

        // Pass 4: 型注釈と明白な演算だけを検査する。Lua の外部値を
        // Unknown として残すため、ランタイム依存のコードは妨げない。
        self.instance_calls.clear();
        self.recording_calls = true;
        self.check_value_types(program);
        self.recording_calls = false;
        // `self` を取るメソッドの呼び出しは、`.` ではなく `:` で呼ぶ。
        if !self.instance_calls.is_empty() {
            crate::method_calls::rewrite(&mut program.stmts, &self.instance_calls);
        }

        // 関数の本体を歩く経路が複数あるため、同じ指摘が重ならないようにする。
        let mut seen = HashSet::new();
        self.errors
            .retain(|error| seen.insert((error.message.clone(), error.line)));
        std::mem::take(&mut self.errors)
    }

    /// `name` に束縛されたテーブルの、トップレベルで確定しているフィールド名。
    /// `!include` したモジュールの未修飾名解決に使う。
    pub fn module_members(&mut self, program: &Program, name: &str) -> Vec<String> {
        self.classes.clear();
        for stmt in &program.stmts {
            if let Stmt::ClassDecl(decl) = stmt {
                self.register_class(decl);
            }
        }
        let mut env = HashMap::new();
        for stmt in &program.stmts {
            self.check_stmt_types(stmt, &mut env);
        }
        self.errors.clear();
        match env.get(name) {
            Some(ValueType::Shape(fields)) => {
                fields.iter().map(|(field, _)| field.clone()).collect()
            }
            _ => Vec::new(),
        }
    }

    fn check_value_types(&mut self, program: &Program) {
        let mut env = self.module_env();
        for stmt in &program.stmts {
            self.check_stmt_types(stmt, &mut env);
        }
    }

    /// import type されたmoduleの宣言から、モジュール名と global の初期の型環境を作る。
    fn module_env(&self) -> HashMap<String, ValueType> {
        let mut env = HashMap::new();
        if let Some(builtins) = &self.builtins {
            for (name, ty) in &builtins.global_types {
                env.insert(name.clone(), self.type_from_annotation(ty));
            }
            for function in &builtins.functions {
                env.insert(function.name.clone(), self.declared_function_type(function, None));
            }
        }
        for module in &self.modules {
            let mut fields: Vec<(String, ValueType)> = module
                .member_types
                .iter()
                .map(|(name, ty)| (name.clone(), self.type_from_annotation(ty)))
                .collect();
            for function in &module.functions {
                let ty = self.declared_function_type(function, Some(&module.name));
                if function.is_global {
                    env.insert(function.name.clone(), ty);
                } else {
                    fields.push((function.name.clone(), ty));
                }
            }
            env.insert(module.name.clone(), ValueType::Shape(fields));
            for (name, ty) in &module.global_types {
                env.insert(name.clone(), self.type_from_annotation(ty));
            }
        }
        env
    }

    fn is_assignable(expected: &ValueType, actual: &ValueType) -> bool {
        match (expected, actual) {
            (_, ValueType::Unknown) | (ValueType::Unknown, _) => true,
            // ユニオンを代入するには、どのメンバーも受け側に入らなければならない。
            (_, ValueType::Union(actuals)) => {
                actuals.iter().all(|actual| Self::is_assignable(expected, actual))
            }
            (ValueType::Union(members), actual) => {
                members.iter().any(|member| Self::is_assignable(member, actual))
            }
            (ValueType::Table | ValueType::Shape(_), ValueType::Table | ValueType::Shape(_)) => true,
            (ValueType::Optional(_), ValueType::Nil) => true,
            (ValueType::Optional(expected), ValueType::Optional(actual)) => {
                Self::is_assignable(expected, actual)
            }
            (ValueType::Optional(inner), actual) => Self::is_assignable(inner, actual),
            (ValueType::Generic(expected), ValueType::Generic(actual)) => expected == actual,
            // クラスのインスタンスはテーブル。
            (ValueType::Table, ValueType::Class(..)) => true,
            (
                ValueType::Record {
                    fields: expected, ..
                },
                ValueType::Shape(actual) | ValueType::Record { fields: actual, .. },
            ) => Self::record_mismatch(expected, actual).is_none(),
            (ValueType::Record { .. }, ValueType::Table)
            | (ValueType::Table | ValueType::Shape(_), ValueType::Record { .. }) => true,
            (
                ValueType::Function | ValueType::FunctionSig(_),
                ValueType::Function | ValueType::FunctionSig(_),
            ) => true,
            (ValueType::Class(expected, expected_args), ValueType::Class(actual, actual_args)) => {
                expected == actual
                    && (expected_args.is_empty()
                        || actual_args.is_empty()
                        || (expected_args.len() == actual_args.len()
                            && expected_args
                                .iter()
                                .zip(actual_args)
                                .all(|(e, a)| Self::is_assignable(e, a))))
            }
            _ => expected == actual,
        }
    }

    fn check_stmt_types(&mut self, stmt: &Stmt, env: &mut HashMap<String, ValueType>) {
        match stmt {
            Stmt::Local {
                names,
                types,
                values,
                line,
            }
            | Stmt::Const {
                names,
                types,
                values,
                line,
            } => {
                for name in names {
                    if matches!(stmt, Stmt::Const { .. }) {
                        self.const_names.insert(name.clone());
                    } else {
                        self.const_names.remove(name);
                    }
                }
                self.validate_binding_types(types, values, *line);
                for (index, name) in names.iter().enumerate() {
                    let actual = values
                        .get(index)
                        .map(|value| self.infer_expr_type(value, env))
                        .unwrap_or(ValueType::Unknown);
                    let declared = types
                        .get(index)
                        .and_then(Option::as_ref)
                        .map(|ty| self.type_from_annotation(ty));
                    if let Some(expected) = declared {
                        if !Self::is_assignable(&expected, &actual) {
                            self.err(
                                format!(
                                    "cannot assign {} to '{}: {}'{}",
                                    actual.display(),
                                    name,
                                    expected.display(),
                                    Self::mismatch_detail(&expected, &actual)
                                ),
                                *line,
                            );
                        }
                        env.insert(name.clone(), expected);
                    } else if actual == ValueType::Unknown
                        && self.modules.iter().any(|module| module.name == *name)
                    {
                        // `local m = require(...)` のように実体を束縛しても、
                        // .luard が宣言した型を Unknown で潰さない。
                    } else {
                        env.insert(name.clone(), actual);
                    }
                }
            }
            Stmt::Assign { targets, values } => {
                for (target, value) in targets.iter().zip(values) {
                    let actual = self.infer_expr_type(value, env);
                    match target {
                        Expr::Ident { name, .. } => {
                            if let Some(expected) = env.get(name).cloned() {
                                if !Self::is_assignable(&expected, &actual) {
                                    self.err(
                                        format!(
                                            "cannot assign {} to '{}: {}'",
                                            actual.display(),
                                            name,
                                            expected.display()
                                        ),
                                        1,
                                    );
                                }
                                // テーブルの再代入では、新しい形を以降の参照へ反映する。
                                if matches!(expected, ValueType::Table | ValueType::Shape(_))
                                    && matches!(actual, ValueType::Shape(_))
                                {
                                    env.insert(name.clone(), actual);
                                }
                            } else {
                                env.insert(name.clone(), actual);
                            }
                        }
                        Expr::Field { obj, name } => {
                            if let Expr::Ident { name: owner, .. } = obj.as_ref() {
                                match env.get_mut(owner) {
                                    Some(ValueType::Shape(fields)) => {
                                        Self::set_shape_field(fields, name, actual);
                                    }
                                    Some(ValueType::Record { fields, .. }) => {
                                        let declared = fields
                                            .iter()
                                            .find(|(field, _)| field == name)
                                            .map(|(_, ty)| ty.clone());
                                        if let Some(expected) = declared {
                                            if !Self::is_assignable(&expected, &actual) {
                                                self.err(
                                                    format!(
                                                        "cannot assign {} to '{owner}.{name}: {}'",
                                                        actual.display(),
                                                        expected.display()
                                                    ),
                                                    1,
                                                );
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Stmt::FunctionDecl {
                name,
                type_params,
                params,
                return_type,
                body,
                line,
                ..
            } => {
                self.validate_function_decl(name, type_params, params, return_type.as_ref(), *line);
                let saved_scope = self.scope_params.len();
                self.scope_params.extend(type_params.iter().cloned());
                // 本体を歩く前に、再帰呼び出しから見える型を入れる(戻り値はまだ分からない)。
                let signature = if type_params.is_empty() {
                    ValueType::Function
                } else {
                    ValueType::FunctionSig(Box::new(self.build_sig(
                        type_params,
                        params,
                        return_type.as_ref(),
                        None,
                    )))
                };
                Self::store_function(env, name, signature);
                let mut child = self.bind_params(env, params);
                self.return_types.push(Vec::new());
                for child_stmt in body {
                    self.check_stmt_types(child_stmt, &mut child);
                }
                let returns = self.return_types.pop().unwrap_or_default();
                if type_params.is_empty() {
                    let inferred = self.inferred_function(params, return_type.as_ref(), returns);
                    Self::store_function(env, name, inferred);
                }
                self.scope_params.truncate(saved_scope);
            }
            Stmt::Do { body } | Stmt::Repeat { body, .. } => {
                if let Stmt::Repeat { cond, .. } = stmt {
                    self.infer_expr_type(cond, env);
                }
                let mut child = env.clone();
                for child_stmt in body {
                    self.check_stmt_types(child_stmt, &mut child);
                }
            }
            Stmt::While { cond, body } => {
                let mut child = env.clone();
                if let Expr::Bind { name, value, .. } = cond {
                    let rhs_type = self.infer_expr_type(value, env);
                    child.insert(name.clone(), Self::truthy_refinement(rhs_type));
                } else {
                    self.infer_expr_type(cond, env);
                }
                for child_stmt in body {
                    self.check_stmt_types(child_stmt, &mut child);
                }
            }
            Stmt::If { clauses, else_body } => {
                for clause in clauses {
                    let mut child = env.clone();
                    if let Expr::Bind { name, value, .. } = &clause.cond {
                        let rhs_type = self.infer_expr_type(value, env);
                        child.insert(name.clone(), Self::truthy_refinement(rhs_type));
                    } else {
                        self.infer_expr_type(&clause.cond, env);
                    }
                    for child_stmt in &clause.body {
                        self.check_stmt_types(child_stmt, &mut child);
                    }
                }
                if let Some(body) = else_body {
                    let mut child = env.clone();
                    for child_stmt in body {
                        self.check_stmt_types(child_stmt, &mut child);
                    }
                }
            }
            Stmt::NumericFor {
                name,
                start,
                limit,
                step,
                body,
            } => {
                self.infer_expr_type(start, env);
                self.infer_expr_type(limit, env);
                if let Some(step) = step {
                    self.infer_expr_type(step, env);
                }
                let mut child = env.clone();
                child.insert(name.clone(), ValueType::Number);
                for child_stmt in body {
                    self.check_stmt_types(child_stmt, &mut child);
                }
            }
            Stmt::GenericFor { names, iters, body } => {
                for iter in iters {
                    self.infer_expr_type(iter, env);
                }
                let mut child = env.clone();
                for name in names {
                    child.insert(name.clone(), ValueType::Unknown);
                }
                for child_stmt in body {
                    self.check_stmt_types(child_stmt, &mut child);
                }
            }
            Stmt::Return(values) => {
                let types: Vec<ValueType> = values
                    .iter()
                    .map(|value| self.infer_expr_type(value, env))
                    .collect();
                // 複数の値を返すときは、型を決めつけない。
                let returned = match types.as_slice() {
                    [] => ValueType::Nil,
                    [single] => single.clone(),
                    _ => ValueType::Unknown,
                };
                if let Some(returns) = self.return_types.last_mut() {
                    returns.push(returned);
                }
            }
            Stmt::ExprStmt(expr) => {
                self.infer_expr_type(expr, env);
            }
            _ => {}
        }
    }

    fn infer_expr_type(&mut self, expr: &Expr, env: &HashMap<String, ValueType>) -> ValueType {
        match expr {
            Expr::Nil => ValueType::Nil,
            Expr::True | Expr::False => ValueType::Boolean,
            Expr::Number(_) => ValueType::Number,
            Expr::Str(_) => ValueType::String,
            Expr::Table(table_fields) => self.infer_table_shape(table_fields, env),
            Expr::Function {
                type_params,
                params,
                return_type,
                body,
            } => self.infer_function_expr(type_params, params, return_type.as_ref(), body, env),
            Expr::Cast { expr, ty, span } => {
                let actual = self.infer_expr_type(expr, env);
                let scope = self.scope_params.clone();
                self.validate_type_with(ty, &scope, None, false, span.line, "in cast: ");
                let target = self.type_from_annotation(ty);
                if !self.cast_allowed(&actual, &target) {
                    self.err_at(
                        format!("cannot cast {} to {}", actual.display(), target.display()),
                        *span,
                    );
                }
                target
            }
            Expr::Ident { name, .. } => env.get(name).cloned().unwrap_or_else(|| {
                if self.classes.contains_key(name) {
                    ValueType::Class(name.clone(), Vec::new())
                } else {
                    ValueType::Unknown
                }
            }),
            Expr::Call { callee, args } => {
                if let Expr::Ident { name, .. } = callee.as_ref() {
                    if name == PROBE_MEMBERS_NAME && args.len() == 1 {
                        let receiver_type = self.infer_expr_type(&args[0], env);
                        let class_object = self.is_class_object(&args[0], env);
                        self.probe_receiver = Some(self.receiver_info(&receiver_type, class_object));
                        self.probe = Some(self.member_items(&receiver_type, class_object));
                        return ValueType::Unknown;
                    }
                    if name == PROBE_SCOPE_NAME && args.is_empty() {
                        self.probe = Some(self.scope_items(env));
                        return ValueType::Unknown;
                    }
                }
                let arg_types: Vec<ValueType> = args
                    .iter()
                    .map(|arg| self.infer_expr_type(arg, env))
                    .collect();
                if let Expr::Field { obj, name } = callee.as_ref() {
                    if name == "new" {
                        if let Expr::Ident { name: class, .. } = obj.as_ref() {
                            if self.classes.contains_key(class) {
                                // `static function new(): Part?` のように戻り値が宣言されていれば、それを使う。
                                let declared = self.method_return_type(
                                    &ValueType::Class(class.clone(), Vec::new()),
                                    "new",
                                );
                                if declared != ValueType::Unknown {
                                    return declared;
                                }
                                return ValueType::Class(class.clone(), Vec::new());
                            }
                        }
                    }
                    let obj_type = self.infer_expr_type(obj, env);
                    if self.recording_calls
                        && !self.is_class_object(obj, env)
                        && self.is_instance_method(&obj_type, name)
                    {
                        self.instance_calls.insert(expr as *const Expr as usize);
                    }
                    if let ValueType::FunctionSig(sig) = self.member_type(&obj_type, name) {
                        return self.check_call(&sig, name, callee, args, &arg_types);
                    }
                    return self.method_return_type(&obj_type, name);
                }
                if let Expr::Ident { name, .. } = callee.as_ref() {
                    if let Some(ValueType::FunctionSig(sig)) = env.get(name) {
                        let sig = sig.clone();
                        return self.check_call(&sig, name, callee, args, &arg_types);
                    }
                }
                if !matches!(callee.as_ref(), Expr::Ident { .. }) {
                    // `f(x)(y)` のように、呼び出し結果を呼ぶ式の内側も検査する。
                    self.infer_expr_type(callee, env);
                }
                ValueType::Unknown
            }
            Expr::MethodCall { obj, method, args } => {
                let obj_type = self.infer_expr_type(obj, env);
                let arg_types: Vec<ValueType> = args
                    .iter()
                    .map(|arg| self.infer_expr_type(arg, env))
                    .collect();
                // `s:upper()` は `string.upper(s)`。レシーバの分だけ最初の引数を省く。
                if obj_type == ValueType::String {
                    if let Some(ValueType::FunctionSig(sig)) = self.builtin_member("string", method) {
                        let mut method_sig = (*sig).clone();
                        if !method_sig.params.is_empty() {
                            method_sig.params.remove(0);
                        }
                        return self.check_call(&method_sig, method, obj, args, &arg_types);
                    }
                }
                self.method_return_type(&obj_type, method)
            }
            Expr::Field { obj, name } => {
                let obj_type = self.infer_expr_type(obj, env);
                self.member_type(&obj_type, name)
            }
            Expr::Index { obj, key } => {
                self.infer_expr_type(obj, env);
                self.infer_expr_type(key, env);
                ValueType::Unknown
            }
            Expr::Unop { op, expr } => {
                let actual = self.infer_expr_type(expr, env);
                if op == "-" && !actual.is_lenient_operand() && actual != ValueType::Number {
                    self.err(
                        format!("unary '-' expects number, got {}", actual.display()),
                        1,
                    );
                }
                if op == "not" {
                    ValueType::Boolean
                } else if op == "-" || op == "#" {
                    ValueType::Number
                } else {
                    ValueType::Unknown
                }
            }
            Expr::Binop {
                op,
                left,
                right,
                span,
            } => {
                let left_type = self.infer_expr_type(left, env);
                let right_type = self.infer_expr_type(right, env);
                self.check_binary_operator(op, &left_type, &right_type, *span);
                match op.as_str() {
                    "+" | "-" | "*" | "/" | "//" | "%" | "^" => ValueType::Number,
                    ".." => ValueType::String,
                    "<" | ">" | "<=" | ">=" | "==" | "~=" => ValueType::Boolean,
                    // `x or default`: 左が偽でなければ左の型、そうでなければ右の型。
                    "or" => Self::or_result_type(left_type, right_type),
                    _ => ValueType::Unknown,
                }
            }
            Expr::InterpolatedString(parts) => {
                for part in parts {
                    if let InterpolatedPart::Expr(expr) = part {
                        self.infer_expr_type(expr, env);
                    }
                }
                ValueType::String
            }
            Expr::If(if_expr) => {
                let mut result_type = None;
                for clause in &if_expr.clauses {
                    let mut branch_env = env.clone();
                    if let Expr::Bind { name, value, .. } = &clause.cond {
                        let rhs_type = self.infer_expr_type(value, env);
                        branch_env.insert(name.clone(), Self::truthy_refinement(rhs_type));
                    } else {
                        self.infer_expr_type(&clause.cond, env);
                    }
                    for statement in &clause.branch.statements {
                        self.check_stmt_types(statement, &mut branch_env);
                    }
                    let branch_type = self.infer_expr_type(&clause.branch.result, &branch_env);
                    result_type =
                        Some(self.merge_if_branch_types(result_type, branch_type, if_expr.span));
                }

                let mut branch_env = env.clone();
                for statement in &if_expr.else_branch.statements {
                    self.check_stmt_types(statement, &mut branch_env);
                }
                let else_type = self.infer_expr_type(&if_expr.else_branch.result, &branch_env);
                self.merge_if_branch_types(result_type, else_type, if_expr.span)
            }
            Expr::Bind { value, .. } => self.infer_expr_type(value, env),
            _ => ValueType::Unknown,
        }
    }

    /// `Dog` のようにクラスそのものを指す識別子か (ローカル変数に隠されていない)。
    fn is_class_object(&self, expr: &Expr, env: &HashMap<String, ValueType>) -> bool {
        matches!(expr, Expr::Ident { name, .. }
            if !env.contains_key(name) && self.classes.contains_key(name))
    }

    /// 補完・ホバー用の型表示。判断できない型は `any`。
    fn describe(&self, ty: &ValueType) -> String {
        match ty {
            ValueType::Unknown => "any".to_string(),
            ValueType::Shape(fields) if !fields.is_empty() => {
                let shown = fields
                    .iter()
                    .take(4)
                    .map(|(name, ty)| format!("{name}: {}", self.describe(ty)))
                    .collect::<Vec<_>>();
                let more = if fields.len() > 4 { ", ..." } else { "" };
                format!("{{ {}{more} }}", shown.join(", "))
            }
            ValueType::Optional(inner) => format!("{}?", self.describe(inner)),
            other => other.display(),
        }
    }

    fn completion_item(label: &str, kind: &str, type_text: String, detail: String) -> CompletionItem {
        CompletionItem {
            label: label.to_string(),
            kind: kind.to_string(),
            type_text,
            detail,
        }
    }

    fn method_detail(method: &MethodMember) -> String {
        let returns = method
            .return_type
            .as_ref()
            .map(|ty| format!(": {}", type_expr_text(ty)))
            .unwrap_or_default();
        format!(
            "{}function {}({}){returns}",
            if method.is_static { "static " } else { "" },
            method.name,
            params_text(&method.params)
        )
    }

    fn member_items(&self, ty: &ValueType, class_object: bool) -> Vec<CompletionItem> {
        match ty {
            ValueType::Shape(fields) | ValueType::Record { fields, .. } => fields
                .iter()
                .map(|(name, field_type)| {
                    let kind = if field_type.is_function() {
                        "function"
                    } else {
                        "field"
                    };
                    let text = self.describe(field_type);
                    Self::completion_item(name, kind, text.clone(), format!("{name}: {text}"))
                })
                .collect(),
            ValueType::Class(class, _) => self.class_items(class, class_object),
            _ => Vec::new(),
        }
    }

    /// クラスの外から見えるメンバー。クラス自身(`Dog.`)ではstaticとコンストラクタ、
    /// インスタンス(`dog.`)ではフィールドとインスタンスメソッドを返す。
    fn class_items(&self, class: &str, class_object: bool) -> Vec<CompletionItem> {
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        let mut declares_new = false;
        let mut current = Some(class.to_string());
        while let Some(class_name) = current {
            if !seen.insert(class_name.clone()) {
                break;
            }
            let Some(info) = self.classes.get(&class_name) else {
                break;
            };
            let mut methods: Vec<_> = info.methods.values().collect();
            methods.sort_by(|a, b| a.method.name.cmp(&b.method.name));
            for method in methods {
                if method.method.name == "new" && method.method.is_static {
                    declares_new = true;
                }
                if method.access != Access::Public || method.method.is_operator {
                    continue;
                }
                let wanted = if class_object {
                    method.method.is_static
                } else {
                    !method.method.is_static && method.method.name != "new"
                };
                if wanted && !items.iter().any(|item: &CompletionItem| item.label == method.method.name) {
                    let return_type = method.method.return_type.as_ref().map(type_expr_text);
                    items.push(Self::completion_item(
                        &method.method.name,
                        "method",
                        return_type.unwrap_or_else(|| "any".to_string()),
                        Self::method_detail(&method.method),
                    ));
                }
            }
            if !class_object {
                let mut fields: Vec<_> = info.fields.values().collect();
                fields.sort_by(|a, b| a.field.name.cmp(&b.field.name));
                for field in fields {
                    if field.access != Access::Public
                        || items.iter().any(|item| item.label == field.field.name)
                    {
                        continue;
                    }
                    let text = self.describe(&self.class_member_type(&class_name, &[], &field.field.name));
                    items.push(Self::completion_item(
                        &field.field.name,
                        "field",
                        text.clone(),
                        format!("{}: {text}", field.field.name),
                    ));
                }
            }
            current = info.parent_name.clone();
        }
        if class_object && !declares_new {
            items.push(Self::completion_item(
                "new",
                "method",
                class.to_string(),
                format!("static function new(): {class}"),
            ));
        }
        items
    }

    /// カーソル位置で見える名前。ローカル・const・関数・モジュール・クラス。
    fn scope_items(&self, env: &HashMap<String, ValueType>) -> Vec<CompletionItem> {
        let mut items = Vec::new();
        let mut names: Vec<_> = env.keys().filter(|name| !name.starts_with("__")).collect();
        names.sort();
        for name in names {
            let ty = &env[name];
            let text = self.describe(ty);
            let is_module = self.modules.iter().any(|module| module.name == *name);
            let (kind, detail) = if is_module {
                ("module", format!("import type {name}"))
            } else if ty.is_function() {
                ("function", format!("function {name}"))
            } else if self.const_names.contains(name) {
                ("constant", format!("const {name}: {text}"))
            } else {
                ("variable", format!("local {name}: {text}"))
            };
            items.push(Self::completion_item(name, kind, text, detail));
        }
        let mut classes: Vec<_> = self.classes.keys().collect();
        classes.sort();
        for name in classes {
            if !env.contains_key(name) {
                items.push(Self::completion_item(
                    name,
                    "class",
                    name.clone(),
                    format!("class {name}"),
                ));
            }
        }
        items
    }

    fn infer_table_shape(
        &mut self,
        table_fields: &[TableField],
        env: &HashMap<String, ValueType>,
    ) -> ValueType {
        let mut fields = Vec::new();
        for field in table_fields {
            match field {
                TableField::Name { name, value } => {
                    let ty = self.infer_expr_type(value, env);
                    Self::set_shape_field(&mut fields, name, ty);
                }
                TableField::Index { key, value } => {
                    self.infer_expr_type(key, env);
                    self.infer_expr_type(value, env);
                }
                TableField::Value(value) => {
                    self.infer_expr_type(value, env);
                }
            }
        }
        ValueType::Shape(fields)
    }

    fn set_shape_field(fields: &mut Vec<(String, ValueType)>, name: &str, ty: ValueType) {
        match fields.iter_mut().find(|(field, _)| field == name) {
            Some(slot) => slot.1 = ty,
            None => fields.push((name.to_string(), ty)),
        }
    }

    /// `obj.name` の型。形が分からないものは Unknown のまま残す。
    fn member_type(&self, obj_type: &ValueType, name: &str) -> ValueType {
        match obj_type {
            ValueType::Shape(fields) => fields
                .iter()
                .find(|(field, _)| field == name)
                .map(|(_, ty)| ty.clone())
                .unwrap_or(ValueType::Unknown),
            ValueType::Record { fields, .. } => fields
                .iter()
                .find(|(field, _)| field == name)
                .map(|(_, ty)| ty.clone())
                .unwrap_or(ValueType::Unknown),
            ValueType::Class(class, args) => self.class_member_type(class, args, name),
            _ => ValueType::Unknown,
        }
    }

    /// クラスの注釈にある型引数 (`T`) を、インスタンスの型引数で置き換えるための束縛。
    /// 継承元のメンバーは、その型引数が分からないので Unknown になる。
    fn class_bindings(
        &self,
        owner: &str,
        instance: &str,
        args: &[ValueType],
    ) -> HashMap<String, ValueType> {
        let Some(info) = self.classes.get(owner) else {
            return HashMap::new();
        };
        info.type_params
            .iter()
            .enumerate()
            .map(|(index, param)| {
                let bound = if owner == instance {
                    args.get(index).cloned().unwrap_or(ValueType::Unknown)
                } else {
                    ValueType::Unknown
                };
                (param.clone(), bound)
            })
            .collect()
    }

    /// クラスのメンバーの注釈を、そのクラスの型引数を見える状態で解決する。
    fn class_annotation(
        &self,
        ty: &TypeExpr,
        owner: &str,
        instance: &str,
        args: &[ValueType],
        method_params: &[String],
    ) -> ValueType {
        let mut params = self
            .classes
            .get(owner)
            .map(|info| info.type_params.clone())
            .unwrap_or_default();
        params.extend(method_params.iter().cloned());
        let ctx = annotation::TypeCtx {
            params: &params,
            module: None,
            depth: 0,
        };
        let resolved = self.resolve_annotation(ty, &ctx);
        let mut bindings = self.class_bindings(owner, instance, args);
        // メソッド自身の型引数は呼び出しからは決まらないので Unknown にする。
        for param in method_params {
            bindings.insert(param.clone(), ValueType::Unknown);
        }
        Self::substitute(&resolved, &bindings)
    }

    fn class_member_type(&self, class: &str, args: &[ValueType], name: &str) -> ValueType {
        if self.lookup_method_in_ancestors(name, class).is_some() {
            return ValueType::Function;
        }
        let Some(field) = self.lookup_field_in_ancestors(name, class) else {
            return ValueType::Unknown;
        };
        if let Some(ty) = &field.field.ty {
            return self.class_annotation(ty, &field.class_name, class, args, &[]);
        }
        match field.field.value.as_ref() {
            Some(Expr::Number(_)) => ValueType::Number,
            Some(Expr::Str(_) | Expr::InterpolatedString(_)) => ValueType::String,
            Some(Expr::True | Expr::False) => ValueType::Boolean,
            _ => ValueType::Unknown,
        }
    }

    /// `obj.name(...)` / `obj:name(...)` の戻り値型。注釈があるときだけ確定する。
    fn method_return_type(&self, obj_type: &ValueType, name: &str) -> ValueType {
        let ValueType::Class(class, args) = obj_type else {
            return ValueType::Unknown;
        };
        let Some(info) = self.lookup_method_in_ancestors(name, class) else {
            return ValueType::Unknown;
        };
        info.method
            .return_type
            .map(|ty| {
                self.class_annotation(&ty, &info.class_name, class, args, &info.method.type_params)
            })
            .unwrap_or(ValueType::Unknown)
    }

    fn truthy_refinement(value_type: ValueType) -> ValueType {
        match value_type {
            ValueType::Optional(inner) => *inner,
            ValueType::Nil => ValueType::Unknown,
            other => other,
        }
    }

    fn merge_if_branch_types(
        &mut self,
        current: Option<ValueType>,
        next: ValueType,
        span: SourceSpan,
    ) -> ValueType {
        let Some(current) = current else {
            return next;
        };
        if Self::is_assignable(&current, &next) {
            return current;
        }
        if Self::is_assignable(&next, &current) {
            return next;
        }
        if current == ValueType::Nil && next != ValueType::Nil {
            return ValueType::Optional(Box::new(next));
        }
        if next == ValueType::Nil && current != ValueType::Nil {
            return ValueType::Optional(Box::new(current));
        }
        if let ValueType::Optional(inner) = &current {
            if Self::is_assignable(inner, &next) {
                return current;
            }
        }
        if let ValueType::Optional(inner) = &next {
            if Self::is_assignable(inner, &current) {
                return next;
            }
        }
        self.err_at(
            format!(
                "if expression branches have incompatible result types: {} and {}",
                current.display(),
                next.display()
            ),
            span,
        );
        ValueType::Unknown
    }

    fn check_binary_operator(
        &mut self,
        op: &str,
        left: &ValueType,
        right: &ValueType,
        span: SourceSpan,
    ) {
        if let ValueType::Class(class_name, _) = left {
            if let Some(info) = self.classes.get(class_name) {
                if let Some(method) = info.methods.get(&format!("operator{op}")) {
                    // `.luard` では `operator+(a: Vector3, b: Vector3)` のように両辺を並べて書ける。
                    let right_param = match method.method.params.as_slice() {
                        [_, second, ..] => Some(second),
                        [first] => Some(first),
                        [] => None,
                    };
                    if let Some(Param::Named {
                        ty: Some(expected), ..
                    }) = right_param
                    {
                        let expected = self.class_annotation(
                            expected,
                            &method.class_name,
                            class_name,
                            &[],
                            &method.method.type_params,
                        );
                        if !Self::is_assignable(&expected, right) {
                            self.err_at(
                                format!(
                                    "operator '{}' for '{}' expects {}, got {}",
                                    op,
                                    class_name,
                                    expected.display(),
                                    right.display()
                                ),
                                span,
                            );
                        }
                    }
                    return;
                }
            }
        }
        let arithmetic = matches!(op, "+" | "-" | "*" | "/" | "//" | "%" | "^");
        if arithmetic
            && !left.is_lenient_operand()
            && !right.is_lenient_operand()
            && (left != &ValueType::Number || right != &ValueType::Number)
        {
            self.err_at(
                format!(
                    "operator '{}' expects number operands, got {} and {}",
                    op,
                    left.display(),
                    right.display()
                ),
                span,
            );
        }
        if op == ".." && !left.is_lenient_operand() && !right.is_lenient_operand() {
            let valid = |ty: &ValueType| matches!(ty, ValueType::String | ValueType::Number);
            if !valid(left) || !valid(right) {
                self.err_at(
                    format!(
                        "operator '..' expects string or number operands, got {} and {}",
                        left.display(),
                        right.display()
                    ),
                    span,
                );
            }
        }
    }

    // ─── Pass 1: Registration ─────────────────────────────────────────────────

    fn register_declared_class(&mut self, class: &DeclaredClass) {
        if let Some(decl) = &class.decl {
            self.register_class(decl);
            return;
        }
        let mut info = ClassInfo {
            name: class.name.clone(),
            type_params: class.type_params.clone(),
            is_abstract: false,
            parent_name: class.parent.clone(),
            methods: HashMap::new(),
            fields: HashMap::new(),
            friends: Vec::new(),
        };
        for (name, ty) in &class.fields {
            info.fields.insert(
                name.clone(),
                FieldInfo {
                    field: FieldMember {
                        name: name.clone(),
                        ty: Some(ty.clone()),
                        value: None,
                    },
                    access: Access::Public,
                    class_name: class.name.clone(),
                },
            );
        }
        for method in &class.methods {
            info.methods.insert(
                method.name.clone(),
                MethodInfo {
                    method: MethodMember {
                        name: method.name.clone(),
                        is_operator: false,
                        operator_op: String::new(),
                        type_params: Vec::new(),
                        is_static: method.is_static,
                        is_abstract: false,
                        is_override: false,
                        is_final: false,
                        params: method.params.clone(),
                        return_type: method.return_type.clone(),
                        body: None,
                    },
                    access: Access::Public,
                    class_name: class.name.clone(),
                },
            );
        }
        self.classes.insert(class.name.clone(), info);
    }

    fn register_class(&mut self, decl: &ClassDecl) {
        if self.classes.contains_key(&decl.name) {
            self.err(
                format!("class '{}' is already defined", decl.name),
                decl.line,
            );
            return;
        }
        let mut info = ClassInfo {
            name: decl.name.clone(),
            type_params: decl.type_params.clone(),
            is_abstract: decl.is_abstract,
            parent_name: decl.parent.clone(),
            methods: HashMap::new(),
            fields: HashMap::new(),
            friends: decl.friends.clone(),
        };
        let mut member_names = HashSet::new();
        for (access, member) in Self::flatten_members(decl) {
            let member_name = match &member {
                Member::Method(m) => &m.name,
                Member::Field(f) => &f.name,
            };
            if !member_names.insert(member_name.clone()) {
                self.err(
                    format!(
                        "member '{}' is defined more than once in class '{}'",
                        member_name, decl.name
                    ),
                    decl.line,
                );
                continue;
            }
            match member {
                Member::Method(m) => {
                    info.methods.insert(
                        m.name.clone(),
                        MethodInfo {
                            method: m,
                            access,
                            class_name: decl.name.clone(),
                        },
                    );
                }
                Member::Field(f) => {
                    info.fields.insert(
                        f.name.clone(),
                        FieldInfo {
                            field: f,
                            access,
                            class_name: decl.name.clone(),
                        },
                    );
                }
            }
        }
        self.classes.insert(decl.name.clone(), info);
    }

    fn flatten_members(decl: &ClassDecl) -> Vec<(Access, Member)> {
        let mut result = Vec::new();
        for m in &decl.top_level_members {
            result.push((Access::Private, m.clone()));
        }
        for block in &decl.blocks {
            for m in &block.members {
                result.push((block.access.clone(), m.clone()));
            }
        }
        result
    }

    // ─── Pass 2: Validation ───────────────────────────────────────────────────

    fn check_class(&mut self, decl: &ClassDecl) {
        self.check_inheritance(decl);
        for friend in &decl.friends {
            if !self.classes.contains_key(friend) {
                self.err(
                    format!("unknown friend class '{}' in class '{}'", friend, decl.name),
                    decl.line,
                );
            }
        }
        let members = Self::flatten_members(decl);
        self.validate_class_annotations(decl, &members);
        let info_clone = self.classes.get(&decl.name).cloned();
        if let Some(info) = info_clone {
            for (access, member) in &members {
                if let Member::Method(m) = member {
                    if m.is_operator && access == &Access::Private {
                        self.err(
                            format!("operator method '{}' must be public", m.name),
                            decl.line,
                        );
                    }
                    self.check_method(m, &info, decl);
                }
            }
        }
        if !decl.is_abstract {
            if let Some(name) = self.first_unimplemented_abstract_method(&decl.name) {
                self.err(
                    format!(
                        "class '{}' must be abstract or implement abstract method '{}'",
                        decl.name, name
                    ),
                    decl.line,
                );
            }
        }
    }

    fn check_inheritance(&mut self, decl: &ClassDecl) {
        let parent = match &decl.parent {
            Some(p) => p.clone(),
            None => return,
        };
        if !self.classes.contains_key(&parent) {
            self.err(format!("unknown parent class '{}'", parent), decl.line);
            return;
        }
        // circular detection
        let mut visited = HashSet::new();
        visited.insert(decl.name.clone());
        let mut current = Some(parent.clone());
        while let Some(cur) = current {
            if visited.contains(&cur) {
                self.err(
                    format!(
                        "circular inheritance detected: '{}' -> '{}'",
                        decl.name, cur
                    ),
                    decl.line,
                );
                return;
            }
            visited.insert(cur.clone());
            current = self.classes.get(&cur).and_then(|i| i.parent_name.clone());
        }
    }

    fn check_method(&mut self, method: &MethodMember, class_info: &ClassInfo, decl: &ClassDecl) {
        let line = decl.line;

        if method.name == "new" && !method.is_static {
            self.err("constructor 'new' must be static".to_string(), line);
        }
        if method.name == "free" && method.is_static {
            self.err("destructor 'free' cannot be static".to_string(), line);
        }
        if method.is_operator && method.is_static {
            self.err(
                format!("operator method '{}' cannot be static", method.name),
                line,
            );
        }
        if method.is_abstract && method.is_final {
            self.err(
                format!("abstract method '{}' cannot be final", method.name),
                line,
            );
        }
        if method.is_abstract && !class_info.is_abstract {
            self.err(
                format!(
                    "method '{}' is abstract but class '{}' is not abstract",
                    method.name, class_info.name
                ),
                line,
            );
        }

        let parent_name = match &class_info.parent_name {
            Some(p) => p.clone(),
            None => {
                if method.is_override {
                    self.err(
                        format!(
                            "method '{}' uses 'override' but class '{}' has no parent",
                            method.name, class_info.name
                        ),
                        line,
                    );
                }
                return;
            }
        };

        let parent_method = self.lookup_method_in_ancestors(&method.name, &parent_name);

        if method.is_override {
            match &parent_method {
                None => {
                    self.err(format!("method '{}' uses 'override' but no such method exists in parent classes", method.name), line);
                }
                Some(pm) => {
                    if pm.method.is_final {
                        self.err(
                            format!(
                                "cannot override final method '{}' from class '{}'",
                                method.name, pm.class_name
                            ),
                            line,
                        );
                    }
                    if !Self::signatures_match(method, &pm.method) {
                        self.err(
                            format!(
                                "override of '{}' does not exactly match the parent signature",
                                method.name
                            ),
                            line,
                        );
                    }
                }
            }
        } else if parent_method.is_some() {
            self.err(
                format!(
                    "method '{}' shadows parent method but is missing 'override' keyword",
                    method.name
                ),
                line,
            );
        }
    }

    fn signatures_match(method: &MethodMember, parent: &MethodMember) -> bool {
        if method.is_static != parent.is_static
            || method.params.len() != parent.params.len()
            || method.return_type != parent.return_type
        {
            return false;
        }

        method
            .params
            .iter()
            .zip(&parent.params)
            .all(|(left, right)| match (left, right) {
                (Param::Vararg, Param::Vararg) => true,
                (Param::Named { ty: left_ty, .. }, Param::Named { ty: right_ty, .. }) => {
                    left_ty == right_ty
                }
                _ => false,
            })
    }

    fn first_unimplemented_abstract_method(&self, class_name: &str) -> Option<String> {
        let mut chain = Vec::new();
        let mut current = Some(class_name.to_string());
        let mut visited = HashSet::new();
        while let Some(name) = current {
            if !visited.insert(name.clone()) {
                return None;
            }
            let info = self.classes.get(&name)?;
            chain.push(info.clone());
            current = info.parent_name.clone();
        }
        chain.reverse();

        let mut methods: HashMap<String, bool> = HashMap::new();
        for info in chain {
            for (name, method) in info.methods {
                methods.insert(name, method.method.is_abstract);
            }
        }
        methods
            .into_iter()
            .find_map(|(name, is_abstract)| is_abstract.then_some(name))
    }

    fn lookup_method_in_ancestors(&self, name: &str, start: &str) -> Option<MethodInfo> {
        let mut current = Some(start.to_string());
        let mut visited = HashSet::new();
        while let Some(class_name) = current {
            if !visited.insert(class_name.clone()) {
                return None;
            }
            let info = self.classes.get(&class_name)?;
            if let Some(m) = info.methods.get(name) {
                return Some(m.clone());
            }
            current = info.parent_name.clone();
        }
        None
    }

    // ─── Pass 3: Access control ───────────────────────────────────────────────

    fn check_access_control(&mut self, program: &Program) {
        let stmts = program.stmts.clone();
        let mut global_env = HashMap::new();
        for stmt in &stmts {
            match stmt {
                Stmt::ClassDecl(decl) => self.check_class_body_access(decl),
                _ => self.check_stmt_access(stmt, &mut global_env, None, false, 0),
            }
        }
    }

    fn check_class_body_access(&mut self, decl: &ClassDecl) {
        let members = Self::flatten_members(decl);
        for (_, member) in members {
            if let Member::Method(m) = member {
                if let Some(body) = &m.body {
                    let mut env = HashMap::new();
                    for p in &m.params {
                        if let Param::Named {
                            name,
                            ty: Some(TypeExpr::Name(type_name) | TypeExpr::Generic { name: type_name, .. }),
                        } = p
                        {
                            if self.classes.contains_key(type_name) {
                                env.insert(name.clone(), type_name.clone());
                            }
                        }
                    }
                    let body = body.clone();
                    let can_use_super = !m.is_static;
                    self.check_body_access(
                        &body,
                        &mut env,
                        Some(&decl.name),
                        can_use_super,
                        decl.line,
                    );
                }
            }
        }
    }

    fn check_body_access(
        &mut self,
        stmts: &[Stmt],
        env: &mut HashMap<String, String>,
        current_class: Option<&str>,
        can_use_super: bool,
        line: usize,
    ) {
        for stmt in stmts {
            self.check_stmt_access(stmt, env, current_class, can_use_super, line);
        }
    }

    fn check_stmt_access(
        &mut self,
        stmt: &Stmt,
        env: &mut HashMap<String, String>,
        current_class: Option<&str>,
        can_use_super: bool,
        line: usize,
    ) {
        match stmt {
            Stmt::Local { names, values, .. } => {
                for v in values {
                    self.check_expr_access(v, env, current_class, can_use_super, line);
                }
                if names.len() == 1 && values.len() == 1 {
                    if let Some(ReceiverKind::Instance(t)) =
                        self.infer_receiver(&values[0], env, current_class)
                    {
                        env.insert(names[0].clone(), t);
                    }
                }
            }
            Stmt::Const { names, values, .. } => {
                for v in values {
                    self.check_expr_access(v, env, current_class, can_use_super, line);
                }
                if names.len() == 1 && values.len() == 1 {
                    if let Some(ReceiverKind::Instance(t)) =
                        self.infer_receiver(&values[0], env, current_class)
                    {
                        env.insert(names[0].clone(), t);
                    }
                }
            }
            Stmt::FunctionDecl { params, body, .. } => {
                let mut function_env = env.clone();
                for param in params {
                    if let Param::Named {
                        name,
                        ty: Some(TypeExpr::Name(type_name) | TypeExpr::Generic { name: type_name, .. }),
                    } = param
                    {
                        if self.classes.contains_key(type_name) {
                            function_env.insert(name.clone(), type_name.clone());
                        }
                    }
                }
                self.check_body_access(body, &mut function_env, current_class, can_use_super, line);
            }
            Stmt::Assign { targets, values } => {
                for e in targets.iter().chain(values.iter()) {
                    self.check_expr_access(e, env, current_class, can_use_super, line);
                }
            }
            Stmt::Return(vals) => {
                for e in vals {
                    self.check_expr_access(e, env, current_class, can_use_super, line);
                }
            }
            Stmt::ExprStmt(e) => {
                self.check_expr_access(e, env, current_class, can_use_super, line);
            }
            Stmt::Do { body } => {
                self.check_body_access(body, &mut env.clone(), current_class, can_use_super, line);
            }
            Stmt::While { cond, body } => {
                self.check_expr_access(cond, env, current_class, can_use_super, line);
                self.check_body_access(body, &mut env.clone(), current_class, can_use_super, line);
            }
            Stmt::Repeat { body, cond } => {
                self.check_body_access(body, &mut env.clone(), current_class, can_use_super, line);
                self.check_expr_access(cond, env, current_class, can_use_super, line);
            }
            Stmt::If { clauses, else_body } => {
                for c in clauses {
                    self.check_expr_access(&c.cond, env, current_class, can_use_super, line);
                    self.check_body_access(
                        &c.body,
                        &mut env.clone(),
                        current_class,
                        can_use_super,
                        line,
                    );
                }
                if let Some(eb) = else_body {
                    self.check_body_access(
                        eb,
                        &mut env.clone(),
                        current_class,
                        can_use_super,
                        line,
                    );
                }
            }
            Stmt::NumericFor {
                start,
                limit,
                step,
                body,
                ..
            } => {
                self.check_expr_access(start, env, current_class, can_use_super, line);
                self.check_expr_access(limit, env, current_class, can_use_super, line);
                if let Some(s) = step {
                    self.check_expr_access(s, env, current_class, can_use_super, line);
                }
                self.check_body_access(body, &mut env.clone(), current_class, can_use_super, line);
            }
            Stmt::GenericFor { iters, body, .. } => {
                for e in iters {
                    self.check_expr_access(e, env, current_class, can_use_super, line);
                }
                self.check_body_access(body, &mut env.clone(), current_class, can_use_super, line);
            }
            _ => {}
        }
    }

    fn check_expr_access(
        &mut self,
        expr: &Expr,
        env: &HashMap<String, String>,
        current_class: Option<&str>,
        can_use_super: bool,
        line: usize,
    ) {
        match expr {
            Expr::Field { obj, name } => {
                if matches!(obj.as_ref(), Expr::SuperExpr) {
                    self.err("'super' must be used as 'super.method()'".to_string(), line);
                    return;
                }
                if let Some(receiver) = self.infer_receiver(obj, env, current_class) {
                    self.check_member_use(&receiver, name, current_class, line);
                }
                self.check_expr_access(obj, env, current_class, can_use_super, line);
            }
            Expr::Call { callee, args } => {
                if let Expr::Field { obj, name } = callee.as_ref() {
                    if matches!(obj.as_ref(), Expr::SuperExpr) {
                        self.check_super_call(name, current_class, can_use_super, line);
                    } else {
                        if let Some(ReceiverKind::Class(class_name)) =
                            self.infer_receiver(obj, env, current_class)
                        {
                            if name == "new"
                                && self
                                    .classes
                                    .get(&class_name)
                                    .is_some_and(|info| info.is_abstract)
                            {
                                self.err(
                                    format!("cannot instantiate abstract class '{}'", class_name),
                                    line,
                                );
                            }
                        }
                        if let Some(receiver) = self.infer_receiver(obj, env, current_class) {
                            self.check_member_use(&receiver, name, current_class, line);
                        }
                        self.check_expr_access(obj, env, current_class, can_use_super, line);
                    }
                } else {
                    self.check_expr_access(callee, env, current_class, can_use_super, line);
                }
                for a in args {
                    self.check_expr_access(a, env, current_class, can_use_super, line);
                }
            }
            Expr::MethodCall { obj, method, args } => {
                if let Some(receiver) = self.infer_receiver(obj, env, current_class) {
                    self.check_member_use(&receiver, method, current_class, line);
                }
                self.check_expr_access(obj, env, current_class, can_use_super, line);
                for a in args {
                    self.check_expr_access(a, env, current_class, can_use_super, line);
                }
            }
            Expr::Index { obj, key } => {
                self.check_expr_access(obj, env, current_class, can_use_super, line);
                self.check_expr_access(key, env, current_class, can_use_super, line);
            }
            Expr::Unop { expr, .. } | Expr::Cast { expr, .. } => {
                self.check_expr_access(expr, env, current_class, can_use_super, line);
            }
            Expr::Binop { left, right, .. } => {
                self.check_expr_access(left, env, current_class, can_use_super, line);
                self.check_expr_access(right, env, current_class, can_use_super, line);
            }
            Expr::Table(fields) => {
                for f in fields {
                    match f {
                        TableField::Value(v) | TableField::Name { value: v, .. } => {
                            self.check_expr_access(v, env, current_class, can_use_super, line);
                        }
                        TableField::Index { key, value } => {
                            self.check_expr_access(key, env, current_class, can_use_super, line);
                            self.check_expr_access(value, env, current_class, can_use_super, line);
                        }
                    }
                }
            }
            Expr::Function { body, .. } => {
                self.check_body_access(body, &mut env.clone(), current_class, can_use_super, line);
            }
            Expr::If(if_expr) => {
                for clause in &if_expr.clauses {
                    self.check_expr_access(&clause.cond, env, current_class, can_use_super, line);
                    let mut branch_env = env.clone();
                    self.check_body_access(
                        &clause.branch.statements,
                        &mut branch_env,
                        current_class,
                        can_use_super,
                        line,
                    );
                    self.check_expr_access(
                        &clause.branch.result,
                        &branch_env,
                        current_class,
                        can_use_super,
                        line,
                    );
                }
                let mut branch_env = env.clone();
                self.check_body_access(
                    &if_expr.else_branch.statements,
                    &mut branch_env,
                    current_class,
                    can_use_super,
                    line,
                );
                self.check_expr_access(
                    &if_expr.else_branch.result,
                    &branch_env,
                    current_class,
                    can_use_super,
                    line,
                );
            }
            Expr::Bind { value, .. } => {
                self.check_expr_access(value, env, current_class, can_use_super, line);
            }
            Expr::SuperExpr => {
                self.err("'super' must be used as 'super.method()'".to_string(), line);
            }
            _ => {}
        }
    }

    fn infer_receiver(
        &self,
        expr: &Expr,
        env: &HashMap<String, String>,
        current_class: Option<&str>,
    ) -> Option<ReceiverKind> {
        match expr {
            Expr::Ident { name: n, .. } => {
                if let Some(class_name) = env.get(n) {
                    Some(ReceiverKind::Instance(class_name.clone()))
                } else if self.classes.contains_key(n) {
                    Some(ReceiverKind::Class(n.clone()))
                } else {
                    None
                }
            }
            Expr::SelfExpr => current_class.map(|name| ReceiverKind::Instance(name.to_string())),
            // `i :: Part` は、`Part` のインスタンスとして扱う。
            Expr::Cast { ty, .. } => match ty {
                TypeExpr::Name(name) | TypeExpr::Generic { name, .. }
                    if self.classes.contains_key(name) =>
                {
                    Some(ReceiverKind::Instance(name.clone()))
                }
                _ => None,
            },
            Expr::Call { callee, .. } => {
                if let Expr::Field { obj, name } = callee.as_ref() {
                    if name == "new" {
                        if let Expr::Ident {
                            name: class_name, ..
                        } = obj.as_ref()
                        {
                            if self.classes.contains_key(class_name) {
                                return Some(ReceiverKind::Instance(class_name.clone()));
                            }
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn lookup_field_in_ancestors(&self, name: &str, start: &str) -> Option<FieldInfo> {
        let mut current = Some(start.to_string());
        let mut visited = HashSet::new();
        while let Some(class_name) = current {
            if !visited.insert(class_name.clone()) {
                return None;
            }
            let info = self.classes.get(&class_name)?;
            if let Some(field) = info.fields.get(name) {
                return Some(field.clone());
            }
            current = info.parent_name.clone();
        }
        None
    }

    /// privateメンバーは、所有クラス自身と、そのクラスが friend に指定したクラスだけが使える。
    fn can_access_private(&self, current_class: Option<&str>, owner: &str) -> bool {
        match current_class {
            Some(current) => {
                current == owner
                    || self
                        .classes
                        .get(owner)
                        .is_some_and(|info| info.friends.iter().any(|friend| friend == current))
            }
            None => false,
        }
    }

    fn check_member_use(
        &mut self,
        receiver: &ReceiverKind,
        member_name: &str,
        current_class: Option<&str>,
        line: usize,
    ) {
        let (class_name, receiver_is_class) = match receiver {
            ReceiverKind::Class(name) => (name, true),
            ReceiverKind::Instance(name) => (name, false),
        };

        if let Some(method) = self.lookup_method_in_ancestors(member_name, class_name) {
            if method.access == Access::Private
                && !self.can_access_private(current_class, &method.class_name)
            {
                self.err(
                    format!(
                        "cannot access private method '{}' of class '{}' from outside",
                        member_name, method.class_name
                    ),
                    line,
                );
            }
            if receiver_is_class && !method.method.is_static {
                self.err(
                    format!(
                        "instance method '{}' cannot be called on class '{}'",
                        member_name, class_name
                    ),
                    line,
                );
            } else if !receiver_is_class && method.method.is_static {
                self.err(
                    format!(
                        "static method '{}' cannot be called on an instance of '{}'",
                        member_name, class_name
                    ),
                    line,
                );
            }
            return;
        }

        if let Some(field) = self.lookup_field_in_ancestors(member_name, class_name) {
            if field.access == Access::Private
                && !self.can_access_private(current_class, &field.class_name)
            {
                self.err(
                    format!(
                        "cannot access private field '{}' of class '{}' from outside",
                        member_name, field.class_name
                    ),
                    line,
                );
            }
            if receiver_is_class {
                self.err(
                    format!(
                        "instance field '{}' cannot be accessed on class '{}'",
                        member_name, class_name
                    ),
                    line,
                );
            }
        }
    }

    fn check_super_call(
        &mut self,
        method_name: &str,
        current_class: Option<&str>,
        can_use_super: bool,
        line: usize,
    ) {
        let Some(class_name) = current_class else {
            self.err("'super' cannot be used outside a class".to_string(), line);
            return;
        };
        if !can_use_super {
            self.err(
                "'super' cannot be used in a static method".to_string(),
                line,
            );
            return;
        }
        let Some(parent_name) = self
            .classes
            .get(class_name)
            .and_then(|info| info.parent_name.clone())
        else {
            self.err(
                format!("class '{}' has no parent for 'super'", class_name),
                line,
            );
            return;
        };
        let Some(parent) = self.classes.get(&parent_name) else {
            return;
        };
        let Some(method) = parent.methods.get(method_name).cloned() else {
            self.err(
                format!(
                    "method '{}' does not exist on direct parent class '{}'",
                    method_name, parent_name
                ),
                line,
            );
            return;
        };
        if method.method.is_static {
            self.err(
                format!(
                    "static parent method '{}' cannot be called through 'super'",
                    method_name
                ),
                line,
            );
        }
        if method.access == Access::Private {
            self.err(
                format!(
                    "cannot access private method '{}' of parent class '{}'",
                    method_name, parent_name
                ),
                line,
            );
        }
    }

    /// 関数の定義を、名前(`a.b` を含む)に応じて型環境へ入れる。
    fn store_function(env: &mut HashMap<String, ValueType>, name: &str, ty: ValueType) {
        match name.split_once('.') {
            Some((owner, member)) if !member.contains('.') => {
                if let Some(ValueType::Shape(fields)) = env.get_mut(owner) {
                    Self::set_shape_field(fields, member, ty);
                }
            }
            Some(_) => {}
            None => {
                env.insert(name.to_string(), ty);
            }
        }
    }

    /// 引数を束縛した、関数本体用の型環境。
    fn bind_params(
        &self,
        env: &HashMap<String, ValueType>,
        params: &[Param],
    ) -> HashMap<String, ValueType> {
        let mut child = env.clone();
        for param in params {
            if let Param::Named { name, ty } = param {
                child.insert(
                    name.clone(),
                    ty.as_ref()
                        .map(|ty| self.type_from_annotation(ty))
                        .unwrap_or(ValueType::Unknown),
                );
            }
        }
        child
    }

    /// `template` のない関数の型。引数は検査せず、戻り値は注釈か `return` から決める。
    fn inferred_function(
        &self,
        params: &[Param],
        return_type: Option<&TypeExpr>,
        returns: Vec<ValueType>,
    ) -> ValueType {
        let ret = match return_type {
            Some(ty) => self.type_from_annotation(ty),
            None => Self::merge_return_types(returns),
        };
        let has_vararg = params.iter().any(|param| matches!(param, Param::Vararg));
        ValueType::FunctionSig(Box::new(FnSig {
            type_params: Vec::new(),
            params: params
                .iter()
                .filter(|param| matches!(param, Param::Named { .. }))
                .map(|_| ValueType::Unknown)
                .collect(),
            vararg: has_vararg.then_some(ValueType::Unknown),
            ret,
            check_args: false,
        }))
    }

    /// 関数式(`function() ... end`、`local function`)の型。本体も歩いて戻り値を推論する。
    fn infer_function_expr(
        &mut self,
        type_params: &[String],
        params: &[Param],
        return_type: Option<&TypeExpr>,
        body: &[Stmt],
        env: &HashMap<String, ValueType>,
    ) -> ValueType {
        let saved_scope = self.scope_params.len();
        self.scope_params.extend(type_params.iter().cloned());
        let mut child = self.bind_params(env, params);
        self.return_types.push(Vec::new());
        for stmt in body {
            self.check_stmt_types(stmt, &mut child);
        }
        let returns = self.return_types.pop().unwrap_or_default();
        let ty = if type_params.is_empty() {
            self.inferred_function(params, return_type, returns)
        } else {
            ValueType::FunctionSig(Box::new(self.build_sig(type_params, params, return_type, None)))
        };
        self.scope_params.truncate(saved_scope);
        ty
    }

    /// 全ての `return` の型を1つにまとめる。食い違えば Unknown。
    fn merge_return_types(returns: Vec<ValueType>) -> ValueType {
        let mut merged: Option<ValueType> = None;
        for next in returns {
            merged = Some(match merged {
                None => next,
                Some(current) if Self::is_assignable(&current, &next) => current,
                Some(current) if Self::is_assignable(&next, &current) => next,
                Some(ValueType::Nil) => ValueType::Optional(Box::new(next)),
                Some(current) if next == ValueType::Nil => ValueType::Optional(Box::new(current)),
                Some(_) => return ValueType::Unknown,
            });
        }
        merged.unwrap_or(ValueType::Nil)
    }

    /// `left or right` の型。
    fn or_result_type(left: ValueType, right: ValueType) -> ValueType {
        let stripped = Self::truthy_refinement(left);
        if stripped == ValueType::Unknown {
            return right;
        }
        if Self::is_assignable(&stripped, &right) {
            stripped
        } else if Self::is_assignable(&right, &stripped) {
            right
        } else {
            ValueType::Unknown
        }
    }

    /// `obj_type` のインスタンスが持つ、公開のインスタンスメソッド `name` か
    /// (`static` と演算子は含まない)。`T?` の受け手は `T` として見る。
    fn is_instance_method(&self, obj_type: &ValueType, name: &str) -> bool {
        let receiver = match obj_type {
            ValueType::Optional(inner) => inner.as_ref(),
            other => other,
        };
        let ValueType::Class(class, _) = receiver else {
            return false;
        };
        self.lookup_method_in_ancestors(name, class).is_some_and(|info| {
            !info.method.is_static && !info.method.is_operator && info.access == Access::Public
        })
    }

    /// 標準ライブラリの `namespace.member` の型。
    fn builtin_member(&self, namespace: &str, member: &str) -> Option<ValueType> {
        let builtins = self.builtins.as_ref()?;
        let (_, ty) = builtins
            .global_types
            .iter()
            .find(|(name, _)| name == namespace)?;
        let TypeExpr::Table(fields) = ty else {
            return None;
        };
        let (_, field) = fields.iter().find(|(name, _)| name == member)?;
        Some(self.type_from_annotation(field))
    }

    fn declared_function_type(
        &self,
        function: &DeclaredFunction,
        module: Option<&str>,
    ) -> ValueType {
        ValueType::FunctionSig(Box::new(self.build_sig(
            &function.type_params,
            &function.params,
            function.return_type.as_ref(),
            module,
        )))
    }

    /// `local x: Box<number>` のような型注釈の型引数の個数などを検査する。
    fn validate_binding_types(
        &mut self,
        types: &[Option<TypeExpr>],
        values: &[Expr],
        line: usize,
    ) {
        let scope = self.scope_params.clone();
        for ty in types.iter().flatten() {
            self.validate_type_with(ty, &scope, None, false, line, "");
        }
        for value in values {
            if let Expr::Function {
                type_params,
                params,
                return_type,
                ..
            } = value
            {
                if !type_params.is_empty() {
                    self.validate_signature(
                        type_params,
                        params,
                        return_type.as_ref(),
                        None,
                        true,
                        line,
                        "in template function: ",
                    );
                }
            }
        }
    }

    /// 関数定義の署名。`template` つきは未定義の型名もエラーにする。
    fn validate_function_decl(
        &mut self,
        name: &str,
        type_params: &[String],
        params: &[Param],
        return_type: Option<&TypeExpr>,
        line: usize,
    ) {
        let prefix = format!("in function '{name}': ");
        self.validate_signature(
            type_params,
            params,
            return_type,
            None,
            !type_params.is_empty(),
            line,
            &prefix,
        );
    }

    /// クラスのフィールド・メソッドの注釈。クラス自身の型引数は見える。
    fn validate_class_annotations(&mut self, decl: &ClassDecl, members: &[(Access, Member)]) {
        let prefix = format!("in class '{}': ", decl.name);
        for (_, member) in members {
            match member {
                Member::Field(field) => {
                    if let Some(ty) = &field.ty {
                        self.validate_type_with(
                            ty,
                            &decl.type_params,
                            None,
                            false,
                            decl.line,
                            &prefix,
                        );
                    }
                }
                Member::Method(method) => self.validate_signature(
                    &[decl.type_params.as_slice(), method.type_params.as_slice()].concat(),
                    &method.params,
                    method.return_type.as_ref(),
                    None,
                    false,
                    decl.line,
                    &prefix,
                ),
            }
        }
    }

    fn err(&mut self, message: String, line: usize) {
        self.errors.push(CheckError {
            message,
            line,
            span: None,
        });
    }

    fn err_at(&mut self, message: String, span: SourceSpan) {
        self.errors.push(CheckError {
            message,
            line: span.line,
            span: Some(span),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn compile_errors(src: &str) -> Vec<String> {
        let mut p = Parser::new(src).unwrap();
        let mut program = p.parse().unwrap();
        Checker::new()
            .check(&mut program)
            .into_iter()
            .map(|e| e.message)
            .collect()
    }

    #[test]
    fn private_new_outside_class_is_error() {
        let src = r#"
class Main is
    static function new()
        print("Hello, Luar!")
    end
end
local lMain = Main.new()
"#;
        let errors = compile_errors(src);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("private") && e.contains("new")),
            "expected private access error for new, got: {:?}",
            errors
        );
    }

    #[test]
    fn public_new_outside_class_is_ok() {
        let src = r#"
class Main is
    public is
        static function new()
            print("Hello, Luar!")
        end
    end
end
local lMain = Main.new()
"#;
        let errors = compile_errors(src);
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }
}
