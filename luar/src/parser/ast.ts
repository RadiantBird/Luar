// ─── Types ───────────────────────────────────────────────────────────────────

export type TypeExpr =
  | { kind: "TypeName"; name: string }
  | { kind: "TypeOptional"; inner: TypeExpr }
  | { kind: "TypeTuple"; types: TypeExpr[] };

// ─── Expressions ─────────────────────────────────────────────────────────────

export type Expr =
  | { kind: "Nil" }
  | { kind: "True" }
  | { kind: "False" }
  | { kind: "Number"; value: string }
  | { kind: "String"; value: string }
  | { kind: "Vararg" }
  | { kind: "Ident"; name: string }
  | { kind: "Self" }
  | { kind: "Super" }
  | { kind: "Field"; obj: Expr; name: string }
  | { kind: "Index"; obj: Expr; key: Expr }
  | { kind: "Call"; callee: Expr; args: Expr[] }
  | { kind: "MethodCall"; obj: Expr; method: string; args: Expr[] }
  | { kind: "Unop"; op: string; expr: Expr }
  | { kind: "Binop"; op: string; left: Expr; right: Expr }
  | { kind: "Table"; fields: TableField[] }
  | { kind: "Function"; params: Param[]; returnType: TypeExpr | null; body: Stmt[] };

export type TableField =
  | { kind: "IndexField"; key: Expr; value: Expr }
  | { kind: "NameField"; name: string; value: Expr }
  | { kind: "ValueField"; value: Expr };

// ─── Statements ──────────────────────────────────────────────────────────────

export type Stmt =
  | { kind: "Local"; names: string[]; types: (TypeExpr | null)[]; values: Expr[] }
  | { kind: "Assign"; targets: Expr[]; values: Expr[] }
  | { kind: "Do"; body: Stmt[] }
  | { kind: "While"; cond: Expr; body: Stmt[] }
  | { kind: "Repeat"; body: Stmt[]; cond: Expr }
  | { kind: "If"; clauses: IfClause[]; elseBody: Stmt[] | null }
  | { kind: "NumericFor"; name: string; start: Expr; limit: Expr; step: Expr | null; body: Stmt[] }
  | { kind: "GenericFor"; names: string[]; iters: Expr[]; body: Stmt[] }
  | { kind: "Return"; values: Expr[] }
  | { kind: "Break" }
  | { kind: "Continue" }
  | { kind: "ExprStmt"; expr: Expr }
  | ClassDecl
  | ImportDecl
  | DeclareStmt;

// import <moduleName>
export type ImportDecl = {
  kind: "ImportDecl";
  moduleName: string;
};

// declare [global] <name>: <type>
export type DeclareStmt = {
  kind: "DeclareStmt";
  isGlobal: boolean;
  name: string;
  type: TypeExpr;
  moduleName: string | null; // どのimportモジュールに属するか（checker/codegenが付与）
};

export type IfClause = { cond: Expr; body: Stmt[] };

// ─── Params ──────────────────────────────────────────────────────────────────

export type Param =
  | { kind: "Param"; name: string; type: TypeExpr | null }
  | { kind: "Vararg" };

// ─── Class declarations ───────────────────────────────────────────────────────

export type AccessMod = "public" | "private";

export type Member = FieldMember | MethodMember;

export type FieldMember = {
  kind: "FieldMember";
  name: string;
  type: TypeExpr | null;
  value: Expr | null;
};

export type MethodMember = {
  kind: "MethodMember";
  name: string;           // "new", "free", "greet", etc.
  isOperator: boolean;    // true for "operator==" etc.
  operatorOp: string;     // the op symbol when isOperator
  isStatic: boolean;
  isAbstract: boolean;
  isOverride: boolean;
  isFinal: boolean;
  params: Param[];
  returnType: TypeExpr | null;
  body: Stmt[] | null;    // null for abstract methods
};

export type MemberBlock = {
  access: AccessMod;
  members: Member[];
};

export type ClassDecl = {
  kind: "ClassDecl";
  name: string;
  isAbstract: boolean;
  parent: string | null;
  topLevelMembers: Member[];   // outside public/private blocks
  blocks: MemberBlock[];
  line: number;
};

// ─── Program ─────────────────────────────────────────────────────────────────

export type Program = {
  kind: "Program";
  stmts: Stmt[];
};
