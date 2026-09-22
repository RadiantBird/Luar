import type {
  Program, Stmt, Expr, TableField, Param,
  ClassDecl, Member, MethodMember, FieldMember,
  ImportDecl, DeclareStmt,
} from "../parser/ast.js";

const OPERATOR_META: Record<string, string> = {
  "==": "__eq",
  "<":  "__lt",
  "<=": "__le",
  "+":  "__add",
  "-":  "__sub",
  "*":  "__mul",
  "/":  "__div",
  "//": "__idiv",
  "%":  "__mod",
  "^":  "__pow",
  "..": "__concat",
  "#":  "__len",
};

// Lightweight per-class method info for dot→colon resolution and new-inheritance
type MethodKind = "instance" | "static" | "operator";
type ClassRegistry = Map<string, {
  methods: Map<string, MethodKind>;
  parent: string | null;
  newParams: Param[] | null;   // params of new() if defined in this class
}>;

export class Codegen {
  private out: string[] = [];
  private indent = 0;

  // Type environment: varName → className (scoped stack)
  private typeEnvStack: Map<string, string>[] = [];
  private currentClass: string | null = null;
  private registry: ClassRegistry = new Map();

  generate(program: Program): string {
    this.out = [];
    this.indent = 0;
    this.typeEnvStack = [new Map()];
    this.registry = this.buildRegistry(program);

    // Emit import collision resolution preamble before any other statements
    this.emitImportPreamble(program);

    for (const stmt of program.stmts) this.emitStmt(stmt);
    return this.out.join("\n");
  }

  // ─── Import preamble ──────────────────────────────────────────────────────

  private emitImportPreamble(program: Program): void {
    // Gather: moduleName → varNames declared by that module
    const moduleVars = new Map<string, string[]>();
    // varName → modules that declare it
    const varModules = new Map<string, string[]>();

    let currentModule: string | null = null;
    for (const stmt of program.stmts) {
      if (stmt.kind === "ImportDecl") {
        currentModule = stmt.moduleName;
        if (!moduleVars.has(currentModule)) moduleVars.set(currentModule, []);
      } else if (stmt.kind === "DeclareStmt" && !stmt.isGlobal && stmt.moduleName !== null) {
        const mod = stmt.moduleName;
        moduleVars.get(mod)?.push(stmt.name) ?? moduleVars.set(mod, [stmt.name]);
        const mods = varModules.get(stmt.name) ?? [];
        mods.push(mod);
        varModules.set(stmt.name, mods);
      }
    }

    // Find all var names that collide across 2+ modules
    const colliding = new Set<string>();
    for (const [varName, mods] of varModules) {
      if (mods.length > 1) colliding.add(varName);
    }

    if (colliding.size === 0) return; // no collisions → no preamble needed

    // Emit LUAR__RAW_ saves for colliding globals
    for (const varName of colliding) {
      this.line(`local LUAR__RAW_${varName} = ${varName}`);
    }
    // Emit per-module namespace tables
    for (const [modName, vars] of moduleVars) {
      const collidingVars = vars.filter(v => colliding.has(v));
      if (collidingVars.length === 0) continue;
      const fields = collidingVars.map(v => `${v} = LUAR__RAW_${v}`).join(", ");
      this.line(`local ${modName} = { ${fields} }`);
    }
  }

  // ─── Class registry (for dot→colon resolution) ────────────────────────────

  private buildRegistry(program: Program): ClassRegistry {
    const reg: ClassRegistry = new Map();
    for (const stmt of program.stmts) {
      if (stmt.kind !== "ClassDecl") continue;
      const methods = new Map<string, MethodKind>();
      for (const block of stmt.blocks) {
        for (const m of block.members) {
          if (m.kind !== "MethodMember") continue;
          if (m.isOperator)  methods.set(m.name, "operator");
          else if (m.isStatic) methods.set(m.name, "static");
          else                 methods.set(m.name, "instance");
        }
      }
      for (const m of stmt.topLevelMembers) {
        if (m.kind !== "MethodMember") continue;
        if (m.isOperator)  methods.set(m.name, "operator");
        else if (m.isStatic) methods.set(m.name, "static");
        else                 methods.set(m.name, "instance");
      }
      // Record new() params for inheritance forwarding
      let newParams: Param[] | null = null;
      for (const block of stmt.blocks) {
        for (const m of block.members) {
          if (m.kind === "MethodMember" && m.name === "new" && m.isStatic) newParams = m.params;
        }
      }
      reg.set(stmt.name, { methods, parent: stmt.parent, newParams });
    }
    return reg;
  }

  private parentName(): string | null {
    return this.currentClass ? (this.registry.get(this.currentClass)?.parent ?? null) : null;
  }

  // Check if an ancestor (anywhere in the chain) defines new()
  // Returns the params of the nearest explicit new, or null if none exists.
  // The call target is always the direct parent — each class delegates up one level.
  private findAncestorNew(parentName: string): { ancestor: string; params: Param[] } | null {
    let current: string | null = parentName;
    while (current) {
      const info = this.registry.get(current);
      if (!info) break;
      if (info.newParams !== null) {
        // Found explicit new params — but always call the DIRECT parent
        return { ancestor: parentName, params: info.newParams };
      }
      current = info.parent;
    }
    return null;
  }

  private emitInheritedNew(
    name: string,
    parentName: string,
    fields: { access: string; member: FieldMember }[]
  ): void {
    const found = this.findAncestorNew(parentName);
    if (!found) return; // no ancestor defines new — class cannot be instantiated

    const { ancestor, params } = found;
    const paramStr = this.emitParams(params);
    const argStr   = params.map(p => p.kind === "Vararg" ? "..." : p.name).join(", ");

    this.line();
    this.line(`function ${name}.new(${paramStr})`);
    this.indented(() => {
      this.line(`local self = ${ancestor}.new(${argStr})`);
      this.line(`setmetatable(self, ${name})`);
      for (const { member: f } of fields) {
        const val = f.value ? this.emitExpr(f.value) : "nil";
        this.line(`self.${f.name} = ${val}`);
      }
      this.line("return self");
    });
    this.line("end");
  }

  // Walk up the inheritance chain to find a method's kind
  private lookupMethod(className: string, methodName: string): MethodKind | null {
    let current: string | null = className;
    while (current) {
      const info = this.registry.get(current);
      if (!info) break;
      const kind = info.methods.get(methodName);
      if (kind !== undefined) return kind;
      current = info.parent;
    }
    return null;
  }

  // ─── Type environment ─────────────────────────────────────────────────────

  private pushScope(): void { this.typeEnvStack.push(new Map()); }
  private popScope(): void  { this.typeEnvStack.pop(); }

  private setType(name: string, className: string): void {
    this.typeEnvStack[this.typeEnvStack.length - 1]!.set(name, className);
  }

  private resolveType(expr: Expr): string | null {
    if (expr.kind === "Ident") {
      for (let i = this.typeEnvStack.length - 1; i >= 0; i--) {
        const t = this.typeEnvStack[i]!.get(expr.name);
        if (t) return t;
      }
      return null;
    }
    // self → current class
    if (expr.kind === "Self") return this.currentClass;
    // ClassName.new(...) → ClassName
    if (expr.kind === "Call" && expr.callee.kind === "Field") {
      const { obj, name } = expr.callee;
      if (name === "new" && obj.kind === "Ident" && this.registry.has(obj.name)) {
        return obj.name;
      }
    }
    return null;
  }

  // ─── Indentation helpers ──────────────────────────────────────────────────

  private line(s = ""): void {
    this.out.push(s === "" ? "" : "    ".repeat(this.indent) + s);
  }

  private indented(fn: () => void): void {
    this.indent++;
    fn();
    this.indent--;
  }

  // ─── Statements ───────────────────────────────────────────────────────────

  private emitStmt(stmt: Stmt): void {
    switch (stmt.kind) {
      case "ClassDecl":    return this.emitClassDecl(stmt);
      case "Local":        return this.emitLocal(stmt);
      case "FunctionDecl": return this.emitFunctionDecl(stmt);
      case "Assign":       return this.emitAssign(stmt);
      case "Do":           this.line("do"); this.indented(() => stmt.body.forEach(s => this.emitStmt(s))); this.line("end"); return;
      case "While":        return this.emitWhile(stmt);
      case "Repeat":       return this.emitRepeat(stmt);
      case "If":           return this.emitIf(stmt);
      case "NumericFor":   return this.emitNumericFor(stmt);
      case "GenericFor":   return this.emitGenericFor(stmt);
      case "Return":       return this.emitReturn(stmt);
      case "Break":        this.line("break"); return;
      case "Continue":     this.line("continue"); return;
      case "Goto":         this.line(`goto ${stmt.label}`); return;
      case "Label":        this.line(`::${stmt.name}::`); return;
      case "ExprStmt":     this.line(this.emitExpr(stmt.expr)); return;
      case "ImportDecl":   return; // preambleで処理済み。Luauランタイムへの実際のrequireは生成しない
      case "DeclareStmt":  return; // 型宣言のみ。コード生成なし
    }
  }

  private emitLocal(stmt: Extract<Stmt, { kind: "Local" }>): void {
    const names = stmt.names.join(", ");
    if (stmt.values.length === 0) {
      this.line(`local ${names}`);
    } else {
      const valStrs = stmt.values.map(v => this.emitExpr(v)).join(", ");
      this.line(`local ${names} = ${valStrs}`);
      // Track type for single assignment: local x = ClassName.new(...)
      if (stmt.names.length === 1 && stmt.values.length === 1) {
        const t = this.resolveType(stmt.values[0]!);
        if (t) this.setType(stmt.names[0]!, t);
      }
    }
  }

  private emitFunctionDecl(stmt: Extract<Stmt, { kind: "FunctionDecl" }>): void {
    this.line(`function ${stmt.name}(${this.emitParams(stmt.params)})`);
    this.indented(() => stmt.body.forEach((child) => this.emitStmt(child)));
    this.line("end");
  }

  private emitAssign(stmt: Extract<Stmt, { kind: "Assign" }>): void {
    const targets = stmt.targets.map(t => this.emitExpr(t)).join(", ");
    const values  = stmt.values.map(v => this.emitExpr(v)).join(", ");
    this.line(`${targets} = ${values}`);
  }

  private emitWhile(stmt: Extract<Stmt, { kind: "While" }>): void {
    this.line(`while ${this.emitExpr(stmt.cond)} do`);
    this.indented(() => stmt.body.forEach(s => this.emitStmt(s)));
    this.line("end");
  }

  private emitRepeat(stmt: Extract<Stmt, { kind: "Repeat" }>): void {
    this.line("repeat");
    this.indented(() => stmt.body.forEach(s => this.emitStmt(s)));
    this.line(`until ${this.emitExpr(stmt.cond)}`);
  }

  private emitIf(stmt: Extract<Stmt, { kind: "If" }>): void {
    stmt.clauses.forEach((clause, i) => {
      const kw = i === 0 ? "if" : "elseif";
      this.line(`${kw} ${this.emitExpr(clause.cond)} then`);
      this.indented(() => clause.body.forEach(s => this.emitStmt(s)));
    });
    if (stmt.elseBody) {
      this.line("else");
      this.indented(() => stmt.elseBody!.forEach(s => this.emitStmt(s)));
    }
    this.line("end");
  }

  private emitNumericFor(stmt: Extract<Stmt, { kind: "NumericFor" }>): void {
    const step = stmt.step ? `, ${this.emitExpr(stmt.step)}` : "";
    this.line(`for ${stmt.name} = ${this.emitExpr(stmt.start)}, ${this.emitExpr(stmt.limit)}${step} do`);
    this.indented(() => stmt.body.forEach(s => this.emitStmt(s)));
    this.line("end");
  }

  private emitGenericFor(stmt: Extract<Stmt, { kind: "GenericFor" }>): void {
    this.line(`for ${stmt.names.join(", ")} in ${stmt.iters.map(i => this.emitExpr(i)).join(", ")} do`);
    this.indented(() => stmt.body.forEach(s => this.emitStmt(s)));
    this.line("end");
  }

  private emitReturn(stmt: Extract<Stmt, { kind: "Return" }>): void {
    if (stmt.values.length === 0) { this.line("return"); return; }
    this.line(`return ${stmt.values.map(v => this.emitExpr(v)).join(", ")}`);
  }

  // ─── Class declaration ────────────────────────────────────────────────────

  private emitClassDecl(decl: ClassDecl): void {
    const name = decl.name;
    const prevClass = this.currentClass;
    this.currentClass = name;

    const allMembers = this.flattenMembers(decl);
    const fields   = allMembers.filter((m): m is { access: string; member: FieldMember }  => m.member.kind === "FieldMember") as { access: string; member: FieldMember }[];
    const methods  = allMembers.filter((m): m is { access: string; member: MethodMember } => m.member.kind === "MethodMember") as { access: string; member: MethodMember }[];

    const privateMethods   = methods.filter(m => m.access === "private");
    const publicMethods    = methods.filter(m => m.access === "public");
    const operatorMethods  = publicMethods.filter(m => m.member.isOperator);
    const normalMethods    = publicMethods.filter(m => !m.member.isOperator);
    const constructorEntry = normalMethods.find(m => m.member.name === "new");
    const destructorEntry  = normalMethods.find(m => m.member.name === "free");
    const otherMethods     = normalMethods.filter(m => m.member.name !== "new" && m.member.name !== "free");

    // Class table
    if (decl.parent) {
      this.line(`local ${name} = setmetatable({}, { __index = ${decl.parent} })`);
    } else {
      this.line(`local ${name} = {}`);
    }
    this.line(`${name}.__index = ${name}`);

    // Operator metamethods
    for (const { member } of operatorMethods) {
      const meta = OPERATOR_META[member.operatorOp] ?? `__${member.operatorOp}`;
      this.line();
      this.line(`${name}.${meta} = function(${this.emitParams([{ kind: "Param", name: "self", type: null }, ...member.params])})`);
      this.indented(() => this.emitBody(member.body ?? []));
      this.line("end");
    }

    // Private methods as local functions
    for (const { member } of privateMethods) {
      if (member.kind !== "MethodMember") continue;
      this.line();
      this.line(`local function ${member.name}(${this.emitParams([{ kind: "Param", name: "self", type: null }, ...member.params])})`);
      this.indented(() => this.emitBody(member.body ?? []));
      this.line("end");
    }

    // Constructor (new) — explicit or auto-inherited
    if (!constructorEntry && decl.parent) {
      this.emitInheritedNew(name, decl.parent, fields);
    }
    if (constructorEntry) {
      const ctor = constructorEntry.member;
      this.line();
      this.line(`function ${name}.new(${this.emitParams(ctor.params)})`);
      this.indented(() => {
        this.line(`local self = setmetatable({}, ${name})`);
        for (const { member: f } of fields) {
          const val = f.value ? this.emitExpr(f.value) : "nil";
          this.line(`self.${f.name} = ${val}`);
        }
        this.emitBody(ctor.body ?? []);
        this.line("return self");
      });
      this.line("end");
    }

    // Static methods (not new/free)
    for (const { member } of otherMethods.filter(m => m.member.isStatic)) {
      this.line();
      this.line(`function ${name}.${member.name}(${this.emitParams(member.params)})`);
      this.indented(() => this.emitBody(member.body ?? []));
      this.line("end");
    }

    // Instance methods (not static, not new/free, not abstract)
    for (const { member } of otherMethods.filter(m => !m.member.isStatic && !m.member.isAbstract)) {
      this.line();
      this.line(`function ${name}.${member.name}(${this.emitParams([{ kind: "Param", name: "self", type: null }, ...member.params])})`);
      this.indented(() => this.emitBody(member.body ?? []));
      this.line("end");
    }

    // Destructor (free)
    if (destructorEntry) {
      const dtor = destructorEntry.member;
      this.line();
      this.line(`function ${name}.free(${this.emitParams([{ kind: "Param", name: "self", type: null }, ...dtor.params])})`);
      this.indented(() => this.emitBody(dtor.body ?? []));
      this.line("end");
    }

    this.currentClass = prevClass;
  }

  // Emit a method body inside its own scope
  private emitBody(stmts: Stmt[]): void {
    this.pushScope();
    stmts.forEach(s => this.emitStmt(s));
    this.popScope();
  }

  private flattenMembers(decl: ClassDecl): { access: string; member: Member }[] {
    const result: { access: string; member: Member }[] = [];
    for (const m of decl.topLevelMembers) result.push({ access: "private", member: m });
    for (const block of decl.blocks) {
      for (const m of block.members) result.push({ access: block.access, member: m });
    }
    return result;
  }

  private emitParams(params: Param[]): string {
    return params.map(p => p.kind === "Vararg" ? "..." : p.name).join(", ");
  }

  // ─── Expressions ──────────────────────────────────────────────────────────

  emitExpr(expr: Expr): string {
    switch (expr.kind) {
      case "Nil":    return "nil";
      case "True":   return "true";
      case "False":  return "false";
      case "Number": return expr.value;
      case "String": return expr.value.startsWith("`") ? expr.value : JSON.stringify(expr.value);
      case "Vararg": return "...";
      case "Ident":  return expr.name;
      case "Self":   return "self";
      case "Super":  return this.parentName() ?? "nil";
      case "Field": {
        // super.field → ParentName.field
        if (expr.obj.kind === "Super") {
          const parent = this.parentName();
          return parent ? `${parent}.${expr.name}` : `nil.${expr.name}`;
        }
        return `${this.emitExpr(expr.obj)}.${expr.name}`;
      }
      case "Index":  return `${this.emitExpr(expr.obj)}[${this.emitExpr(expr.key)}]`;

      case "Call": {
        const argStrs = expr.args.map(a => this.emitExpr(a)).join(", ");
        if (expr.callee.kind === "Field") {
          const { obj, name } = expr.callee;
          // super.method(args) → ParentName.method(self, args)
          if (obj.kind === "Super") {
            const parent = this.parentName();
            if (parent) {
              const allArgs = argStrs ? `self, ${argStrs}` : "self";
              return `${parent}.${name}(${allArgs})`;
            }
          }
          // dot→colon: instance method call
          const objType = this.resolveType(obj);
          if (objType) {
            const kind = this.lookupMethod(objType, name);
            if (kind === "instance") {
              return `${this.emitExpr(obj)}:${name}(${argStrs})`;
            }
          }
        }
        return `${this.emitExpr(expr.callee)}(${argStrs})`;
      }

      case "MethodCall": {
        const argStrs = expr.args.map(a => this.emitExpr(a)).join(", ");
        return `${this.emitExpr(expr.obj)}:${expr.method}(${argStrs})`;
      }

      case "Unop":  return `${expr.op} ${this.emitExpr(expr.expr)}`;
      case "Binop": return `${this.emitExpr(expr.left)} ${expr.op} ${this.emitExpr(expr.right)}`;
      case "Table": return this.emitTable(expr.fields);

      case "Function": {
        const saved = this.out;
        this.out = [];
        this.indented(() => {
          this.pushScope();
          expr.body.forEach(s => this.emitStmt(s));
          this.popScope();
        });
        const body = this.out.join("\n");
        this.out = saved;
        return `function(${this.emitParams(expr.params)})\n${body}\n${"    ".repeat(this.indent)}end`;
      }
    }
  }

  private emitTable(fields: TableField[]): string {
    if (fields.length === 0) return "{}";
    const parts = fields.map(f => {
      switch (f.kind) {
        case "IndexField": return `[${this.emitExpr(f.key)}] = ${this.emitExpr(f.value)}`;
        case "NameField":  return `${f.name} = ${this.emitExpr(f.value)}`;
        case "ValueField": return this.emitExpr(f.value);
      }
    });
    return `{ ${parts.join(", ")} }`;
  }
}
