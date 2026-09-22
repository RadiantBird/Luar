import type { Program, Stmt, Expr, ClassDecl, MethodMember, FieldMember, Member, AccessMod, ImportDecl, DeclareStmt } from "../parser/ast.js";
import type { CheckError, ClassInfo, MethodInfo, FieldInfo } from "./types.js";

export { type CheckError };

// varName → [moduleName, ...] (複数モジュールが同じ名前をdeclareしている場合は衝突)
type ModuleRegistry = Map<string, string[]>;

export class Checker {
  private errors: CheckError[] = [];
  private classes = new Map<string, ClassInfo>();
  // importされたモジュール名のセット
  private importedModules = new Set<string>();
  // モジュールのdeclare情報: varName → [moduleName...]
  private moduleRegistry: ModuleRegistry = new Map();

  check(program: Program): CheckError[] {
    this.errors = [];
    this.classes = new Map();
    this.importedModules = new Set();
    this.moduleRegistry = new Map();

    // Pass 0: collect imports and declares
    this.collectImports(program);

    // Pass 1: collect all class declarations into registry
    for (const stmt of program.stmts) {
      if (stmt.kind === "ClassDecl") this.registerClass(stmt);
    }

    // Pass 2: validate each class
    for (const stmt of program.stmts) {
      if (stmt.kind === "ClassDecl") this.checkClass(stmt);
    }

    // Pass 3: access control
    this.checkAccessControl(program);

    return this.errors;
  }

  // ─── Pass 0: Import / Declare collection ──────────────────────────────────

  private collectImports(program: Program): void {
    // 1st: collect import statements
    for (const stmt of program.stmts) {
      if (stmt.kind === "ImportDecl") {
        if (this.importedModules.has(stmt.moduleName)) {
          this.error(`module '${stmt.moduleName}' is imported more than once`, 0, 1);
        }
        this.importedModules.add(stmt.moduleName);
      }
    }

    // 2nd: collect declare statements and associate them with the preceding import context
    // DeclareStmts without a corresponding import are treated as global ambient declarations
    let currentModule: string | null = null;
    for (const stmt of program.stmts) {
      if (stmt.kind === "ImportDecl") {
        currentModule = stmt.moduleName;
      } else if (stmt.kind === "DeclareStmt") {
        const mod = stmt.isGlobal ? null : currentModule;
        // Patch moduleName onto the AST node for codegen
        (stmt as DeclareStmt).moduleName = mod;
        if (mod !== null) {
          const existing = this.moduleRegistry.get(stmt.name) ?? [];
          existing.push(mod);
          this.moduleRegistry.set(stmt.name, existing);
        }
      }
    }

    // 3rd: detect ambiguous unqualified names (same varName declared by 2+ modules, both non-global)
    for (const [varName, mods] of this.moduleRegistry) {
      if (mods.length > 1) {
        this.error(
          `ambiguous unqualified name '${varName}': declared in modules [${mods.join(", ")}]. Use qualified access (e.g. ${mods[0]}.${varName})`,
          0, 1
        );
      }
    }
  }

  // ─── Pass 1: Registration ──────────────────────────────────────────────────

  private registerClass(decl: ClassDecl): void {
    if (this.classes.has(decl.name)) {
      this.error(`class '${decl.name}' is already defined`, decl.line, 1);
      return;
    }

    const info: ClassInfo = {
      name: decl.name,
      isAbstract: decl.isAbstract,
      parentName: decl.parent,
      methods: new Map(),
      fields: new Map(),
      line: decl.line,
    };

    const allMembers = this.flattenMembers(decl);
    for (const { access, member } of allMembers) {
      if (member.kind === "MethodMember") {
        info.methods.set(member.name, { method: member, access, className: decl.name });
      } else {
        info.fields.set(member.name, { field: member, access, className: decl.name });
      }
    }

    this.classes.set(decl.name, info);
  }

  private flattenMembers(decl: ClassDecl): { access: AccessMod; member: Member }[] {
    const result: { access: AccessMod; member: Member }[] = [];
    // Top-level members default to private per LPL.md spec
    for (const m of decl.topLevelMembers) {
      result.push({ access: "private", member: m });
    }
    for (const block of decl.blocks) {
      for (const m of block.members) {
        result.push({ access: block.access, member: m });
      }
    }
    return result;
  }

  // ─── Pass 2: Validation ────────────────────────────────────────────────────

  private checkClass(decl: ClassDecl): void {
    const info = this.classes.get(decl.name)!;

    this.checkInheritance(decl);

    const allMembers = this.flattenMembers(decl);
    for (const { member } of allMembers) {
      if (member.kind === "MethodMember") {
        this.checkMethod(member, info, decl);
        if (member.body) this.checkBodyForSuper(member.body, info, decl);
      }
    }
  }

  private checkInheritance(decl: ClassDecl): void {
    if (!decl.parent) return;

    // Parent must exist
    if (!this.classes.has(decl.parent)) {
      this.error(`unknown parent class '${decl.parent}'`, decl.line, 1);
      return;
    }

    // Circular inheritance detection
    const visited = new Set<string>([decl.name]);
    let current: string | null = decl.parent;
    while (current) {
      if (visited.has(current)) {
        this.error(`circular inheritance detected: '${decl.name}' -> '${current}'`, decl.line, 1);
        return;
      }
      visited.add(current);
      current = this.classes.get(current)?.parentName ?? null;
    }
  }

  private checkMethod(method: MethodMember, classInfo: ClassInfo, decl: ClassDecl): void {
    const line = decl.line;

    // operator cannot be static
    if (method.isOperator && method.isStatic) {
      this.error(
        `operator method '${method.name}' cannot be static`,
        line, 1
      );
    }

    // abstract method requires abstract class
    if (method.isAbstract && !classInfo.isAbstract) {
      this.error(
        `method '${method.name}' is abstract but class '${classInfo.name}' is not abstract`,
        line, 1
      );
    }

    if (!classInfo.parentName) {
      // No parent: override/final with no parent is meaningless
      if (method.isOverride) {
        this.error(
          `method '${method.name}' uses 'override' but class '${classInfo.name}' has no parent`,
          line, 1
        );
      }
      return;
    }

    const parentMethod = this.lookupMethodInAncestors(method.name, classInfo.parentName);

    if (method.isOverride) {
      // override requires parent to have the method
      if (!parentMethod) {
        this.error(
          `method '${method.name}' uses 'override' but no such method exists in parent classes`,
          line, 1
        );
        return;
      }

      // cannot override a final method
      if (parentMethod.method.isFinal) {
        this.error(
          `cannot override final method '${method.name}' from class '${parentMethod.className}'`,
          line, 1
        );
      }

      // signature must match: same number of params
      const parentParams = parentMethod.method.params.filter(p => p.kind === "Param").length;
      const thisParams   = method.params.filter(p => p.kind === "Param").length;
      if (parentParams !== thisParams) {
        this.error(
          `override of '${method.name}' has ${thisParams} param(s) but parent has ${parentParams}`,
          line, 1
        );
      }

      // return type must match when both are annotated
      const parentRet = parentMethod.method.returnType;
      const thisRet   = method.returnType;
      if (parentRet && thisRet && !this.typeExprEqual(parentRet, thisRet)) {
        this.error(
          `override of '${method.name}' has return type '${this.typeExprStr(thisRet)}' but parent has '${this.typeExprStr(parentRet)}'`,
          line, 1
        );
      }
    } else {
      // No override keyword: parent must NOT have the same method
      if (parentMethod) {
        this.error(
          `method '${method.name}' shadows parent method but is missing 'override' keyword`,
          line, 1
        );
      }
    }
  }

  // ─── Pass 3: Access control ───────────────────────────────────────────────

  private checkAccessControl(program: Program): void {
    const globalEnv = new Map<string, string>();
    for (const stmt of program.stmts) {
      if (stmt.kind === "ClassDecl") {
        this.checkClassBodyAccess(stmt);
      } else {
        this.checkStmtAccess(stmt, globalEnv, null);
      }
    }
  }

  private checkClassBodyAccess(decl: ClassDecl): void {
    for (const { member } of this.flattenMembers(decl)) {
      if (member.kind === "MethodMember" && member.body) {
        const env = new Map<string, string>();
        // Pre-populate env with parameter types from annotations (e.g. f: Foo)
        for (const p of member.params) {
          if (p.kind === "Param" && p.type?.kind === "TypeName" && this.classes.has(p.type.name)) {
            env.set(p.name, p.type.name);
          }
        }
        this.checkBodyAccess(member.body, env, decl.name, decl.line);
      }
    }
  }

  private checkBodyAccess(stmts: Stmt[], env: Map<string, string>, currentClass: string | null, line: number): void {
    for (const stmt of stmts) this.checkStmtAccess(stmt, env, currentClass, line);
  }

  private checkStmtAccess(stmt: Stmt, env: Map<string, string>, currentClass: string | null, line = 0): void {
    switch (stmt.kind) {
      case "Local": {
        stmt.values.forEach(v => this.checkExprAccess(v, env, currentClass, line));
        if (stmt.names.length === 1 && stmt.values.length === 1) {
          const t = this.inferType(stmt.values[0]!, env, currentClass);
          if (t) env.set(stmt.names[0]!, t);
        }
        break;
      }
      case "FunctionDecl":
        this.checkBodyAccess(stmt.body, new Map(env), currentClass, line);
        break;
      case "Assign":
        [...stmt.targets, ...stmt.values].forEach(e => this.checkExprAccess(e, env, currentClass, line));
        break;
      case "Return":     stmt.values.forEach(e => this.checkExprAccess(e, env, currentClass, line)); break;
      case "ExprStmt":   this.checkExprAccess(stmt.expr, env, currentClass, line); break;
      case "Do":         this.checkBodyAccess(stmt.body, new Map(env), currentClass, line); break;
      case "While":      this.checkExprAccess(stmt.cond, env, currentClass, line); this.checkBodyAccess(stmt.body, new Map(env), currentClass, line); break;
      case "Repeat":     this.checkBodyAccess(stmt.body, new Map(env), currentClass, line); this.checkExprAccess(stmt.cond, env, currentClass, line); break;
      case "If":
        stmt.clauses.forEach(c => { this.checkExprAccess(c.cond, env, currentClass, line); this.checkBodyAccess(c.body, new Map(env), currentClass, line); });
        if (stmt.elseBody) this.checkBodyAccess(stmt.elseBody, new Map(env), currentClass, line);
        break;
      case "NumericFor":
        [stmt.start, stmt.limit, ...(stmt.step ? [stmt.step] : [])].forEach(e => this.checkExprAccess(e, env, currentClass, line));
        this.checkBodyAccess(stmt.body, new Map(env), currentClass, line);
        break;
      case "GenericFor":
        stmt.iters.forEach(e => this.checkExprAccess(e, env, currentClass, line));
        this.checkBodyAccess(stmt.body, new Map(env), currentClass, line);
        break;
    }
  }

  private checkExprAccess(expr: Expr, env: Map<string, string>, currentClass: string | null, line: number): void {
    switch (expr.kind) {
      case "Field": {
        const objType = this.inferType(expr.obj, env, currentClass);
        if (objType) this.assertMemberAccess(objType, expr.name, currentClass, line);
        this.checkExprAccess(expr.obj, env, currentClass, line);
        break;
      }
      case "Call": {
        // field call: expr.method(args) — check access on the field name
        if (expr.callee.kind === "Field") {
          const { obj, name } = expr.callee;
          // ClassName.new() — disallow instantiation of abstract classes
          if (name === "new" && obj.kind === "Ident") {
            const classInfo = this.classes.get(obj.name);
            if (classInfo?.isAbstract) {
              this.error(`cannot instantiate abstract class '${obj.name}'`, line, 1);
            }
          }
          const objType = this.inferType(obj, env, currentClass);
          if (objType) this.assertMemberAccess(objType, name, currentClass, line);
          this.checkExprAccess(obj, env, currentClass, line);
        } else {
          this.checkExprAccess(expr.callee, env, currentClass, line);
        }
        expr.args.forEach(a => this.checkExprAccess(a, env, currentClass, line));
        break;
      }
      case "MethodCall":  this.checkExprAccess(expr.obj, env, currentClass, line); expr.args.forEach(a => this.checkExprAccess(a, env, currentClass, line)); break;
      case "Index":       this.checkExprAccess(expr.obj, env, currentClass, line); this.checkExprAccess(expr.key, env, currentClass, line); break;
      case "Unop":        this.checkExprAccess(expr.expr, env, currentClass, line); break;
      case "Binop":       this.checkExprAccess(expr.left, env, currentClass, line); this.checkExprAccess(expr.right, env, currentClass, line); break;
      case "Table":
        expr.fields.forEach(f => {
          if (f.kind === "ValueField") this.checkExprAccess(f.value, env, currentClass, line);
          else { if (f.kind === "IndexField") this.checkExprAccess(f.key, env, currentClass, line); this.checkExprAccess(f.value, env, currentClass, line); }
        });
        break;
      case "Function":    this.checkBodyAccess(expr.body, new Map(env), currentClass, line); break;
      default: break;
    }
  }

  // Resolve the class type of an expression (best-effort)
  private inferType(expr: Expr, env: Map<string, string>, currentClass: string | null): string | null {
    if (expr.kind === "Ident")  return env.get(expr.name) ?? null;
    if (expr.kind === "Self")   return currentClass;
    // ClassName.new(...) → ClassName
    if (expr.kind === "Call" && expr.callee.kind === "Field") {
      const { obj, name } = expr.callee;
      if (name === "new" && obj.kind === "Ident" && this.classes.has(obj.name)) return obj.name;
    }
    return null;
  }

  // Check that accessing memberName on className is allowed from currentClass
  private assertMemberAccess(className: string, memberName: string, currentClass: string | null, line: number): void {
    const classInfo = this.classes.get(className);
    if (!classInfo) return;

    const method = classInfo.methods.get(memberName);
    const field  = classInfo.fields.get(memberName);
    const info   = method ?? field;
    if (!info) return; // unknown member — not our job here

    // Access is allowed if: public, OR accessed from the same class
    if (info.access === "private" && currentClass !== className) {
      const kind = method ? "method" : "field";
      this.error(
        `cannot access private ${kind} '${memberName}' of class '${className}' from outside`,
        line, 1
      );
    }
  }

  // ─── super validation ─────────────────────────────────────────────────────

  private checkBodyForSuper(stmts: Stmt[], classInfo: ClassInfo, decl: ClassDecl): void {
    for (const stmt of stmts) this.checkStmtForSuper(stmt, classInfo, decl);
  }

  private checkStmtForSuper(stmt: Stmt, classInfo: ClassInfo, decl: ClassDecl): void {
    switch (stmt.kind) {
      case "Local":      stmt.values.forEach(e => this.checkExprForSuper(e, classInfo, decl)); break;
      case "FunctionDecl": this.checkBodyForSuper(stmt.body, classInfo, decl); break;
      case "Assign":     [...stmt.targets, ...stmt.values].forEach(e => this.checkExprForSuper(e, classInfo, decl)); break;
      case "Return":     stmt.values.forEach(e => this.checkExprForSuper(e, classInfo, decl)); break;
      case "ExprStmt":   this.checkExprForSuper(stmt.expr, classInfo, decl); break;
      case "If":
        stmt.clauses.forEach(c => { this.checkExprForSuper(c.cond, classInfo, decl); this.checkBodyForSuper(c.body, classInfo, decl); });
        if (stmt.elseBody) this.checkBodyForSuper(stmt.elseBody, classInfo, decl);
        break;
      case "While":      this.checkExprForSuper(stmt.cond, classInfo, decl); this.checkBodyForSuper(stmt.body, classInfo, decl); break;
      case "Repeat":     this.checkBodyForSuper(stmt.body, classInfo, decl); this.checkExprForSuper(stmt.cond, classInfo, decl); break;
      case "NumericFor": [stmt.start, stmt.limit, ...(stmt.step ? [stmt.step] : [])].forEach(e => this.checkExprForSuper(e, classInfo, decl)); this.checkBodyForSuper(stmt.body, classInfo, decl); break;
      case "GenericFor": stmt.iters.forEach(e => this.checkExprForSuper(e, classInfo, decl)); this.checkBodyForSuper(stmt.body, classInfo, decl); break;
      case "Do":         this.checkBodyForSuper(stmt.body, classInfo, decl); break;
    }
  }

  private checkExprForSuper(expr: Expr, classInfo: ClassInfo, decl: ClassDecl): void {
    switch (expr.kind) {
      case "Super": {
        // bare super (not as part of a field access) is always invalid
        this.error(`'super' must be used as 'super.method()'`, decl.line, 1);
        break;
      }
      case "Call": {
        // super.method(args) — the only valid super usage
        if (expr.callee.kind === "Field" && expr.callee.obj.kind === "Super") {
          this.validateSuperCall(expr.callee.name, classInfo, decl);
          expr.args.forEach(a => this.checkExprForSuper(a, classInfo, decl));
          return;
        }
        this.checkExprForSuper(expr.callee, classInfo, decl);
        expr.args.forEach(a => this.checkExprForSuper(a, classInfo, decl));
        break;
      }
      case "Field": {
        if (expr.obj.kind === "Super") {
          // super.field as non-call — treat as super.method check
          this.validateSuperCall(expr.name, classInfo, decl);
          return;
        }
        this.checkExprForSuper(expr.obj, classInfo, decl);
        break;
      }
      case "Index":      this.checkExprForSuper(expr.obj, classInfo, decl); this.checkExprForSuper(expr.key, classInfo, decl); break;
      case "MethodCall": this.checkExprForSuper(expr.obj, classInfo, decl); expr.args.forEach(a => this.checkExprForSuper(a, classInfo, decl)); break;
      case "Unop":       this.checkExprForSuper(expr.expr, classInfo, decl); break;
      case "Binop":      this.checkExprForSuper(expr.left, classInfo, decl); this.checkExprForSuper(expr.right, classInfo, decl); break;
      case "Table":
        expr.fields.forEach(f => {
          if (f.kind === "IndexField") { this.checkExprForSuper(f.key, classInfo, decl); this.checkExprForSuper(f.value, classInfo, decl); }
          else if (f.kind === "NameField" || f.kind === "ValueField") this.checkExprForSuper(f.value, classInfo, decl);
        });
        break;
      case "Function":   this.checkBodyForSuper(expr.body, classInfo, decl); break;
      default: break; // literals, Ident, Self, Vararg — no super
    }
  }

  private validateSuperCall(methodName: string, classInfo: ClassInfo, decl: ClassDecl): void {
    // super requires a parent class
    if (!classInfo.parentName) {
      this.error(
        `'super.${methodName}' used in class '${classInfo.name}' which has no parent`,
        decl.line, 1
      );
      return;
    }

    // The method must exist in the direct parent (not ancestors — super only refers to direct parent)
    const parentInfo = this.classes.get(classInfo.parentName);
    if (parentInfo && !parentInfo.methods.has(methodName)) {
      this.error(
        `'super.${methodName}' does not exist in parent class '${classInfo.parentName}'`,
        decl.line, 1
      );
    }
  }

  // ─── Type helpers ─────────────────────────────────────────────────────────

  private typeExprEqual(a: import("../parser/ast.js").TypeExpr, b: import("../parser/ast.js").TypeExpr): boolean {
    if (a.kind !== b.kind) return false;
    if (a.kind === "TypeName"     && b.kind === "TypeName")     return a.name === b.name;
    if (a.kind === "TypeOptional" && b.kind === "TypeOptional") return this.typeExprEqual(a.inner, b.inner);
    if (a.kind === "TypeTuple"    && b.kind === "TypeTuple")
      return a.types.length === b.types.length && a.types.every((t, i) => this.typeExprEqual(t, b.types[i]!));
    return false;
  }

  private typeExprStr(t: import("../parser/ast.js").TypeExpr): string {
    if (t.kind === "TypeName")     return t.name;
    if (t.kind === "TypeOptional") return `${this.typeExprStr(t.inner)}?`;
    if (t.kind === "TypeTuple")    return `(${t.types.map(x => this.typeExprStr(x)).join(", ")})`;
    return "unknown";
  }

  // Walk up the inheritance chain to find a method by name
  private lookupMethodInAncestors(name: string, startClassName: string): MethodInfo | null {
    let current: string | null = startClassName;
    while (current) {
      const info = this.classes.get(current);
      if (!info) break;
      const m = info.methods.get(name);
      if (m) return m;
      current = info.parentName;
    }
    return null;
  }

  private error(message: string, line: number, col: number): void {
    this.errors.push({ message, line, col });
  }
}
