import { Lexer } from "../lexer/lexer.js";
import type { Token, TokenKind } from "../lexer/token.js";
import type {
  Program, Stmt, Expr, Param, TypeExpr, TableField,
  ClassDecl, MemberBlock, Member, FieldMember, MethodMember,
  IfClause, AccessMod, ImportDecl, DeclareStmt, FunctionDecl,
} from "./ast.js";

export class ParseError extends Error {
  constructor(msg: string, public line: number, public col: number) {
    super(`[${line}:${col}] ${msg}`);
    this.name = "ParseError";
  }
}

export class Parser {
  private tokens: Token[];
  private pos = 0;

  constructor(src: string) {
    this.tokens = new Lexer(src).tokenize();
  }

  // ─── Helpers ───────────────────────────────────────────────────────────────

  private peek(): Token { return this.tokens[this.pos]!; }

  private check(...kinds: TokenKind[]): boolean {
    return kinds.includes(this.peek().kind);
  }

  private advance(): Token { return this.tokens[this.pos++]!; }

  private eat(kind: TokenKind): Token {
    if (!this.check(kind)) {
      const t = this.peek();
      throw new ParseError(`expected '${kind}', got '${t.kind}'`, t.line, t.col);
    }
    return this.advance();
  }

  private eatIdent(): string { return this.eat("Ident").value; }

  private match(...kinds: TokenKind[]): boolean {
    if (this.check(...kinds)) { this.advance(); return true; }
    return false;
  }

  private isAtEnd(): boolean { return this.peek().kind === "EOF"; }

  // ─── Public entry ──────────────────────────────────────────────────────────

  parse(): Program {
    const stmts = this.parseBlock(["EOF"]);
    return { kind: "Program", stmts };
  }

  // ─── Block & statements ────────────────────────────────────────────────────

  private parseBlock(terminators: TokenKind[]): Stmt[] {
    const stmts: Stmt[] = [];
    while (!this.isAtEnd() && !this.check(...terminators)) {
      if (this.match(";")) continue;
      stmts.push(this.parseStmt());
    }
    return stmts;
  }

  private parseStmt(): Stmt {
    const t = this.peek();

    switch (t.kind) {
      case "class":   return this.parseClassDecl();
      case "local":   return this.parseLocal();
      case "function": return this.parseFunctionDecl();
      case "do":      return this.parseDo();
      case "while":   return this.parseWhile();
      case "repeat":  return this.parseRepeat();
      case "if":      return this.parseIf();
      case "for":     return this.parseFor();
      case "return":  return this.parseReturn();
      case "break":   this.advance(); return { kind: "Break" };
      case "continue":this.advance(); return { kind: "Continue" };
      case "goto": {
        const token = this.advance();
        return { kind: "Goto", label: this.eatIdent(), line: token.line, col: token.col };
      }
      case ":": {
        const start = this.advance();
        this.eat(":");
        const name = this.eatIdent();
        this.eat(":");
        this.eat(":");
        return { kind: "Label", name, line: start.line, col: start.col };
      }
      case "import":  return this.parseImport();
      case "declare": return this.parseDeclare();
      default:        return this.parseExprOrAssign();
    }
  }

  private parseLocal(): Stmt {
    this.eat("local");

    // Lua-compatible named local function declaration:
    //   local function name(...) ... end
    if (this.match("function")) {
      const name = this.eatIdent();
      const { params, returnType } = this.parseFuncSignature();
      const body = this.parseFuncBody();
      return {
        kind: "Local",
        names: [name],
        types: [null],
        values: [{ kind: "Function", params, returnType, body }],
      };
    }

    const names: string[] = [];
    const types: (TypeExpr | null)[] = [];
    names.push(this.eatIdent());
    types.push(this.tryParseTypeAnnotation());
    while (this.match(",")) {
      names.push(this.eatIdent());
      types.push(this.tryParseTypeAnnotation());
    }
    let values: Expr[] = [];
    if (this.match("=")) values = this.parseExprList();
    return { kind: "Local", names, types, values };
  }

  private parseFunctionDecl(): FunctionDecl {
    this.eat("function");
    let name = this.eatIdent();
    while (this.match(".")) name += `.${this.eatIdent()}`;
    const { params, returnType } = this.parseFuncSignature();
    const body = this.parseFuncBody();
    return { kind: "FunctionDecl", name, params, returnType, body };
  }

  private parseDo(): Stmt {
    this.eat("do");
    const body = this.parseBlock(["end"]);
    this.eat("end");
    return { kind: "Do", body };
  }

  private parseWhile(): Stmt {
    this.eat("while");
    const cond = this.parseExpr();
    this.eat("do");
    const body = this.parseBlock(["end"]);
    this.eat("end");
    return { kind: "While", cond, body };
  }

  private parseRepeat(): Stmt {
    this.eat("repeat");
    const body = this.parseBlock(["until"]);
    this.eat("until");
    const cond = this.parseExpr();
    return { kind: "Repeat", body, cond };
  }

  private parseIf(): Stmt {
    this.eat("if");
    const clauses: IfClause[] = [];
    const cond = this.parseExpr();
    this.eat("then");
    const body = this.parseBlock(["elseif", "else", "end"]);
    clauses.push({ cond, body });
    while (this.match("elseif")) {
      const c = this.parseExpr();
      this.eat("then");
      const b = this.parseBlock(["elseif", "else", "end"]);
      clauses.push({ cond: c, body: b });
    }
    let elseBody: Stmt[] | null = null;
    if (this.match("else")) {
      elseBody = this.parseBlock(["end"]);
    }
    this.eat("end");
    return { kind: "If", clauses, elseBody };
  }

  private parseFor(): Stmt {
    this.eat("for");
    const first = this.eatIdent();
    if (this.match("=")) {
      // numeric for
      const start = this.parseExpr();
      this.eat(",");
      const limit = this.parseExpr();
      const step = this.match(",") ? this.parseExpr() : null;
      this.eat("do");
      const body = this.parseBlock(["end"]);
      this.eat("end");
      return { kind: "NumericFor", name: first, start, limit, step, body };
    } else {
      // generic for
      const names = [first];
      while (this.match(",")) names.push(this.eatIdent());
      this.eat("in");
      const iters = this.parseExprList();
      this.eat("do");
      const body = this.parseBlock(["end"]);
      this.eat("end");
      return { kind: "GenericFor", names, iters, body };
    }
  }

  private parseReturn(): Stmt {
    this.eat("return");
    const values: Expr[] = [];
    if (!this.isAtEnd() && !this.check("end", "else", "elseif", "until", "EOF")) {
      values.push(...this.parseExprList());
    }
    this.match(";");
    return { kind: "Return", values };
  }

  private parseExprOrAssign(): Stmt {
    const exprs = [this.parseSuffixExpr()];
    while (this.match(",")) exprs.push(this.parseSuffixExpr());

    if (this.match("=")) {
      const values = this.parseExprList();
      return { kind: "Assign", targets: exprs, values };
    }

    if (exprs.length === 1) return { kind: "ExprStmt", expr: exprs[0]! };

    const t = this.peek();
    throw new ParseError("expected assignment", t.line, t.col);
  }

  // ─── Class declarations ────────────────────────────────────────────────────

  private parseClassDecl(): ClassDecl {
    const { line, col } = this.peek();
    this.eat("class");
    const name = this.eatIdent();
    this.eat("is");

    let isAbstract = false;
    let parent: string | null = null;

    if (this.match("abstract")) {
      isAbstract = true;
    } else if (this.check("Ident")) {
      parent = this.eatIdent();
    }

    const topLevelMembers: Member[] = [];
    const blocks: MemberBlock[] = [];

    while (!this.check("end") && !this.isAtEnd()) {
      if (this.check("public") || this.check("private")) {
        blocks.push(this.parseMemberBlock());
      } else {
        topLevelMembers.push(this.parseMember());
      }
    }

    this.eat("end");
    return { kind: "ClassDecl", name, isAbstract, parent, topLevelMembers, blocks, line };
  }

  private parseMemberBlock(): MemberBlock {
    const access = this.advance().kind as AccessMod;
    this.eat("is");
    const members: Member[] = [];
    while (!this.check("end") && !this.isAtEnd()) {
      members.push(this.parseMember());
    }
    this.eat("end");
    return { access, members };
  }

  private parseMember(): Member {
    // Only "static" is a leading modifier in Luar
    const isStatic = this.match("static");

    // operator overload: function operator==(...)
    if (this.check("function")) {
      const next2 = this.tokens[this.pos + 1];
      if (next2?.kind === "operator") {
        return this.parseOperatorMethod(isStatic);
      }
      return this.parseMethod(isStatic);
    }

    // Field: Name [: Type] [= expr]
    const name = this.eatIdent();
    const type = this.tryParseTypeAnnotation();
    let value: Expr | null = null;
    if (this.match("=")) value = this.parseExpr();
    return { kind: "FieldMember", name, type, value };
  }

  // abstract / override / final appear after the closing paren, before the body
  private parseTrailingModifiers(): { isAbstract: boolean; isOverride: boolean; isFinal: boolean } {
    let isAbstract = false, isOverride = false, isFinal = false;
    while (true) {
      if (this.match("abstract")) { isAbstract = true; continue; }
      if (this.match("override")) { isOverride = true; continue; }
      if (this.match("final"))    { isFinal = true;    continue; }
      break;
    }
    return { isAbstract, isOverride, isFinal };
  }

  private parseMethod(isStatic: boolean): MethodMember {
    this.eat("function");
    const name = this.eatIdent();
    const { params, returnType } = this.parseFuncSignature();
    const { isAbstract, isOverride, isFinal } = this.parseTrailingModifiers();
    const body = isAbstract ? null : this.parseFuncBody();
    return { kind: "MethodMember", name, isOperator: false, operatorOp: "", isStatic, isAbstract, isOverride, isFinal, params, returnType, body };
  }

  private parseOperatorMethod(isStatic: boolean): MethodMember {
    this.eat("function");
    this.eat("operator");
    // consume the operator symbol (one or two tokens)
    const op = this.advance().value;
    const { params, returnType } = this.parseFuncSignature();
    const { isAbstract, isOverride, isFinal } = this.parseTrailingModifiers();
    const body = isAbstract ? null : this.parseFuncBody();
    return { kind: "MethodMember", name: `operator${op}`, isOperator: true, operatorOp: op, isStatic, isAbstract, isOverride, isFinal, params, returnType, body };
  }

  private parseFuncSignature(): { params: Param[]; returnType: TypeExpr | null } {
    this.eat("(");
    const params: Param[] = [];
    if (!this.check(")")) {
      params.push(...this.parseParamList());
    }
    this.eat(")");
    const returnType = this.match(":") ? this.parseTypeExpr() : null;
    return { params, returnType };
  }

  private parseFuncBody(): Stmt[] {
    const body = this.parseBlock(["end"]);
    this.eat("end");
    return body;
  }

  private parseParamList(): Param[] {
    const params: Param[] = [];
    if (this.match("...")) {
      params.push({ kind: "Vararg" });
      return params;
    }
    const name = this.eatIdent();
    const type = this.tryParseTypeAnnotation();
    params.push({ kind: "Param", name, type });
    while (this.match(",")) {
      if (this.match("...")) { params.push({ kind: "Vararg" }); break; }
      const n = this.eatIdent();
      const ty = this.tryParseTypeAnnotation();
      params.push({ kind: "Param", name: n, type: ty });
    }
    return params;
  }

  // ─── Type annotations ──────────────────────────────────────────────────────

  private tryParseTypeAnnotation(): TypeExpr | null {
    if (!this.match(":")) return null;
    return this.parseTypeExpr();
  }

  private parseTypeExpr(): TypeExpr {
    const name = this.eatIdent();
    const inner: TypeExpr = { kind: "TypeName", name };
    if (this.match("?")) return { kind: "TypeOptional", inner };
    return inner;
  }

  // ─── Expressions ──────────────────────────────────────────────────────────

  private parseExprList(): Expr[] {
    const exprs = [this.parseExpr()];
    while (this.match(",")) exprs.push(this.parseExpr());
    return exprs;
  }

  private parseExpr(): Expr { return this.parseOr(); }

  private parseOr(): Expr {
    let left = this.parseAnd();
    while (this.match("or")) left = { kind: "Binop", op: "or", left, right: this.parseAnd() };
    return left;
  }

  private parseAnd(): Expr {
    let left = this.parseComparison();
    while (this.match("and")) left = { kind: "Binop", op: "and", left, right: this.parseComparison() };
    return left;
  }

  private parseComparison(): Expr {
    let left = this.parseConcat();
    while (this.check("<", ">", "<=", ">=", "==", "~=")) {
      const op = this.advance().value;
      left = { kind: "Binop", op, left, right: this.parseConcat() };
    }
    return left;
  }

  private parseConcat(): Expr {
    const left = this.parseAddSub();
    if (this.match("..")) return { kind: "Binop", op: "..", left, right: this.parseConcat() };
    return left;
  }

  private parseAddSub(): Expr {
    let left = this.parseMulDiv();
    while (this.check("+", "-")) {
      const op = this.advance().value;
      left = { kind: "Binop", op, left, right: this.parseMulDiv() };
    }
    return left;
  }

  private parseMulDiv(): Expr {
    let left = this.parseUnary();
    while (this.check("*", "/", "//", "%")) {
      const op = this.advance().value;
      left = { kind: "Binop", op, left, right: this.parseUnary() };
    }
    return left;
  }

  private parseUnary(): Expr {
    if (this.match("not")) return { kind: "Unop", op: "not", expr: this.parseUnary() };
    if (this.match("-"))   return { kind: "Unop", op: "-",   expr: this.parseUnary() };
    if (this.match("#"))   return { kind: "Unop", op: "#",   expr: this.parseUnary() };
    return this.parsePower();
  }

  private parsePower(): Expr {
    const base = this.parseSuffixExpr();
    if (this.match("^")) return { kind: "Binop", op: "^", left: base, right: this.parseUnary() };
    return base;
  }

  private parseSuffixExpr(): Expr {
    let expr = this.parsePrimaryExpr();
    while (true) {
      if (this.match(".")) {
        const name = this.eatIdent();
        expr = { kind: "Field", obj: expr, name };
      } else if (this.check("[")) {
        this.advance();
        const key = this.parseExpr();
        this.eat("]");
        expr = { kind: "Index", obj: expr, key };
      } else if (this.check("(")) {
        const args = this.parseCallArgs();
        expr = { kind: "Call", callee: expr, args };
      } else if (this.check(":")) {
        this.advance();
        const method = this.eatIdent();
        const args = this.parseCallArgs();
        expr = { kind: "MethodCall", obj: expr, method, args };
      } else {
        break;
      }
    }
    return expr;
  }

  private parseCallArgs(): Expr[] {
    this.eat("(");
    if (this.match(")")) return [];
    const args = this.parseExprList();
    this.eat(")");
    return args;
  }

  private parsePrimaryExpr(): Expr {
    const t = this.peek();
    switch (t.kind) {
      case "nil":     this.advance(); return { kind: "Nil" };
      case "true":    this.advance(); return { kind: "True" };
      case "false":   this.advance(); return { kind: "False" };
      case "Number":  this.advance(); return { kind: "Number", value: t.value };
      case "String":  this.advance(); return { kind: "String", value: t.value };
      case "...":     this.advance(); return { kind: "Vararg" };
      case "self":    this.advance(); return { kind: "Self" };
      case "super":   this.advance(); return { kind: "Super" };
      case "Ident":   this.advance(); return { kind: "Ident", name: t.value };
      case "function": return this.parseFunctionExpr();
      case "{":       return this.parseTableConstructor();
      case "(": {
        this.advance();
        const expr = this.parseExpr();
        this.eat(")");
        return expr;
      }
      default:
        throw new ParseError(`unexpected token '${t.kind}'`, t.line, t.col);
    }
  }

  private parseFunctionExpr(): Expr {
    this.eat("function");
    const { params, returnType } = this.parseFuncSignature();
    const body = this.parseFuncBody();
    return { kind: "Function", params, returnType, body };
  }

  private parseTableConstructor(): Expr {
    this.eat("{");
    const fields: TableField[] = [];
    while (!this.check("}") && !this.isAtEnd()) {
      if (this.match(";") || this.match(",")) continue;
      fields.push(this.parseTableField());
    }
    this.eat("}");
    return { kind: "Table", fields };
  }

  private parseTableField(): TableField {
    if (this.check("[")) {
      this.advance();
      const key = this.parseExpr();
      this.eat("]");
      this.eat("=");
      const value = this.parseExpr();
      return { kind: "IndexField", key, value };
    }
    if (this.check("Ident") && this.tokens[this.pos + 1]?.kind === "=") {
      const name = this.eatIdent();
      this.eat("=");
      const value = this.parseExpr();
      return { kind: "NameField", name, value };
    }
    return { kind: "ValueField", value: this.parseExpr() };
  }

  // ─── import / declare ────────────────────────────────────────────────────

  private parseImport(): ImportDecl {
    this.eat("import");
    if (!this.check("Ident") || this.peek().value !== "type") {
      const token = this.peek();
      throw new ParseError("expected 'type' after 'import'; use 'import type <module>'", token.line, token.col);
    }
    this.advance();
    const moduleName = this.eatIdent();
    return { kind: "ImportDecl", moduleName };
  }

  private parseDeclare(): DeclareStmt {
    this.eat("declare");
    const isGlobal = this.match("global");
    const name = this.eatIdent();
    this.eat(":");
    const type = this.parseTypeExpr();
    return { kind: "DeclareStmt", isGlobal, name, type, moduleName: null };
  }
}
