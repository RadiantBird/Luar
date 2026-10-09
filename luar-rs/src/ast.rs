#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    /// `number` や `mod.MyTable` のような型名。修飾名はドット込みの1つの文字列。
    Name(String),
    /// `MyTable<number>` のような型引数つきの型名。
    Generic { name: String, args: Vec<TypeExpr> },
    Optional(Box<TypeExpr>),
    Tuple(Vec<TypeExpr>),
    /// `{ id: number, ref: T }`。宣言順。
    Table(Vec<(String, TypeExpr)>),
    /// `{ T }`。要素の型が揃った配列。
    Array(Box<TypeExpr>),
    /// 関数型の最後の引数 `...T`。
    Vararg(Box<TypeExpr>),
    /// ユニオン型 `A | B`。
    Union(Vec<TypeExpr>),
    /// `(A, B) -> R`
    Function {
        params: Vec<TypeExpr>,
        ret: Box<TypeExpr>,
    },
}

impl std::fmt::Display for TypeExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeExpr::Name(name) => write!(f, "{name}"),
            TypeExpr::Generic { name, args } => write!(f, "{name}<{}>", join_types(args)),
            TypeExpr::Optional(inner) => match inner.as_ref() {
                TypeExpr::Union(_) | TypeExpr::Function { .. } => write!(f, "({inner})?"),
                _ => write!(f, "{inner}?"),
            },
            TypeExpr::Union(members) => {
                let members = members
                    .iter()
                    .map(|member| match member {
                        TypeExpr::Function { .. } => format!("({member})"),
                        _ => member.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(" | ");
                write!(f, "{members}")
            }
            TypeExpr::Tuple(types) => write!(f, "({})", join_types(types)),
            TypeExpr::Table(fields) => {
                let fields = fields
                    .iter()
                    .map(|(name, ty)| format!("{name}: {ty}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "{{ {fields} }}")
            }
            TypeExpr::Array(element) => write!(f, "{{ {element} }}"),
            TypeExpr::Vararg(element) => write!(f, "...{element}"),
            TypeExpr::Function { params, ret } => {
                write!(f, "({}) -> {ret}", join_types(params))
            }
        }
    }
}

fn join_types(types: &[TypeExpr]) -> String {
    types
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug, Clone)]
pub enum Expr {
    Nil,
    True,
    False,
    Number(String),
    Str(String),
    InterpolatedString(Vec<InterpolatedPart>),
    Vararg,
    Ident {
        name: String,
        span: SourceSpan,
    },
    SelfExpr,
    SuperExpr,
    Field {
        obj: Box<Expr>,
        name: String,
    },
    Index {
        obj: Box<Expr>,
        key: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    MethodCall {
        obj: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
    Unop {
        op: String,
        expr: Box<Expr>,
    },
    Binop {
        op: String,
        left: Box<Expr>,
        right: Box<Expr>,
        /// 演算子そのものの位置。型エラーを式の先頭ではなく演算子へ表示する。
        span: SourceSpan,
    },
    Table(Vec<TableField>),
    Function {
        type_params: Vec<String>,
        params: Vec<Param>,
        return_type: Option<TypeExpr>,
        body: Vec<Stmt>,
    },
    /// A block-valued conditional expression.  Each branch keeps its
    /// statements separate from the expression whose value it produces.
    If(IfExpr),
    /// 型キャスト `expr :: Type`。`span` は `::` の位置。
    Cast {
        expr: Box<Expr>,
        ty: TypeExpr,
        span: SourceSpan,
    },
    /// A condition-only local binding: `name := value`.
    Bind {
        name: String,
        value: Box<Expr>,
        span: SourceSpan,
    },
}

#[derive(Debug, Clone)]
pub struct IfExpr {
    pub clauses: Vec<IfExprClause>,
    pub else_branch: IfExprBranch,
    pub span: SourceSpan,
}

#[derive(Debug, Clone)]
pub struct IfExprClause {
    pub cond: Expr,
    pub branch: IfExprBranch,
}

#[derive(Debug, Clone)]
pub struct IfExprBranch {
    pub statements: Vec<Stmt>,
    pub result: Box<Expr>,
}

#[derive(Debug, Clone)]
pub enum InterpolatedPart {
    Literal(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum TableField {
    Index { key: Expr, value: Expr },
    Name { name: String, value: Expr },
    Value(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Param {
    Named { name: String, ty: Option<TypeExpr> },
    Vararg,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Local {
        names: Vec<String>,
        types: Vec<Option<TypeExpr>>,
        values: Vec<Expr>,
        /// 宣言由来の型不一致を、実際の宣言行へ報告するために保持する。
        line: usize,
    },
    Const {
        names: Vec<String>,
        types: Vec<Option<TypeExpr>>,
        values: Vec<Expr>,
        line: usize,
    },
    FunctionDecl {
        name: String,
        type_params: Vec<String>,
        params: Vec<Param>,
        return_type: Option<TypeExpr>,
        body: Vec<Stmt>,
        is_const: bool,
        line: usize,
    },
    Assign {
        targets: Vec<Expr>,
        values: Vec<Expr>,
    },
    Do {
        body: Vec<Stmt>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    Repeat {
        body: Vec<Stmt>,
        cond: Expr,
    },
    If {
        clauses: Vec<IfClause>,
        else_body: Option<Vec<Stmt>>,
    },
    NumericFor {
        name: String,
        start: Expr,
        limit: Expr,
        step: Option<Expr>,
        body: Vec<Stmt>,
    },
    GenericFor {
        names: Vec<String>,
        iters: Vec<Expr>,
        body: Vec<Stmt>,
    },
    Return(Vec<Expr>),
    Break,
    Continue,
    Goto {
        label: String,
        line: usize,
    },
    Label {
        name: String,
        line: usize,
    },
    /// Lua 5.4 source validated by full_moon and preserved for the Lua backend.
    RawLua54(String),
    ExprStmt(Expr),
    ClassDecl(ClassDecl),
    ImportDecl {
        module_name: String,
        /// `import type name from "path"` の `path`。なければ同じディレクトリの `name.luard`。
        path: Option<String>,
    },
    DeclareStmt {
        is_global: bool,
        name: String,
        ty: TypeExpr,
        module_name: Option<String>,
    },
    /// `[export] type Name<T> = ...`。実行時コードは生成しない。
    TypeAlias {
        is_export: bool,
        name: String,
        type_params: Vec<String>,
        ty: TypeExpr,
        line: usize,
    },
    /// `declare [global] function name(params): ret`(.luard専用)。
    DeclareFunction {
        is_global: bool,
        name: String,
        type_params: Vec<String>,
        params: Vec<Param>,
        return_type: Option<TypeExpr>,
        line: usize,
    },
}

#[derive(Debug, Clone)]
pub struct IfClause {
    pub cond: Expr,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub struct ClassDecl {
    pub name: String,
    pub type_params: Vec<String>,
    pub is_abstract: bool,
    pub parent: Option<String>,
    pub top_level_members: Vec<Member>,
    pub blocks: Vec<MemberBlock>,
    /// friend class X で、privateメンバーへのアクセスを許可したクラス名。
    pub friends: Vec<String>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct MemberBlock {
    pub access: Access,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Access {
    Public,
    Private,
}

#[derive(Debug, Clone)]
pub enum Member {
    Field(FieldMember),
    Method(MethodMember),
}

#[derive(Debug, Clone)]
pub struct FieldMember {
    pub name: String,
    pub ty: Option<TypeExpr>,
    pub value: Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct MethodMember {
    pub name: String,
    /// `template <T> function ...` のメソッド自身の型引数。
    pub type_params: Vec<String>,
    pub is_operator: bool,
    pub operator_op: String,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_override: bool,
    pub is_final: bool,
    pub params: Vec<Param>,
    pub return_type: Option<TypeExpr>,
    pub body: Option<Vec<Stmt>>,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub stmts: Vec<Stmt>,
}
use crate::lexer::SourceSpan;
