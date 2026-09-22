import { Lexer } from "../../luar/src/lexer/lexer";
import type { Token } from "../../luar/src/lexer/token";
import * as fs from "node:fs";
import * as path from "node:path";
import { Parser } from "../../luar/src/parser/parser";
import type {
  ClassDecl,
  Expr,
  Member,
  MethodMember,
  Param,
  Program,
  Stmt,
  TypeExpr,
} from "../../luar/src/parser/ast";

export type SymbolKind = "class" | "method" | "field" | "function" | "variable" | "module";

export interface LanguageSymbol {
  name: string;
  kind: SymbolKind;
  signature: string;
  line: number;
  col: number;
  endLine: number;
  endCol: number;
  parent?: string;
}

export interface DocumentIndex {
  symbols: LanguageSymbol[];
  classes: Map<string, LanguageSymbol>;
  members: Map<string, LanguageSymbol[]>;
  keywords: string[];
}

export interface ModuleDeclaration {
  name: string;
  isGlobal: boolean;
  typeText: string;
  line: number;
  col: number;
  endLine: number;
  endCol: number;
}

export interface ModuleDefinition {
  moduleName: string;
  filePath: string;
  declarations: ModuleDeclaration[];
  members: ModuleDeclaration[];
  globals: ModuleDeclaration[];
}

export interface ModuleDefinitionError {
  message: string;
  line: number;
  col: number;
}

export interface ModuleDefinitionResult {
  definition: ModuleDefinition | null;
  errors: ModuleDefinitionError[];
}

/** Parse the deliberately small .luard grammar used by the Rust compiler. */
export function parseModuleDefinition(moduleName: string, source: string, filePath = `${moduleName}.luard`): ModuleDefinitionResult {
  const errors: ModuleDefinitionError[] = [];
  let tokens: Token[];
  try {
    tokens = new Lexer(source).tokenize();
  } catch (error) {
    errors.push(moduleError(filePath, 1, 1, errorMessage(error)));
    return { definition: null, errors };
  }

  const declarations: ModuleDeclaration[] = [];
  const names = new Set<string>();
  let pos = 0;
  while (tokens[pos]?.kind !== "EOF") {
    const declare = tokens[pos];
    if (!declare || declare.kind !== "declare") {
      errors.push(moduleError(filePath, declare?.line ?? 1, declare?.col ?? 1, "expected 'declare'"));
      break;
    }
    pos++;
    const isGlobal = tokens[pos]?.kind === "global";
    if (isGlobal) pos++;
    const name = tokens[pos];
    if (!name || name.kind !== "Ident") {
      errors.push(moduleError(filePath, name?.line ?? declare.line, name?.col ?? declare.col, "expected identifier"));
      pos = nextDeclaration(tokens, pos);
      continue;
    }
    pos++;
    const colon = tokens[pos];
    if (!colon || colon.kind !== ":") {
      errors.push(moduleError(filePath, colon?.line ?? name.line, colon?.col ?? name.col, "expected ':'"));
      pos = nextDeclaration(tokens, pos);
      continue;
    }
    pos++;
    const typeStart = pos;
    const typeEnd = parseDefinitionType(tokens, pos);
    if (typeEnd < 0) {
      const bad = tokens[typeStart] ?? colon;
      errors.push(moduleError(filePath, bad.line, bad.col, "expected type"));
      pos = nextDeclaration(tokens, typeStart);
      continue;
    }
    pos = typeEnd;
    const lastTypeToken = tokens[pos - 1]!;
    const typeText = sourceTextBetween(source, tokens[typeStart]!, lastTypeToken);
    const declaration: ModuleDeclaration = {
      name: name.value,
      isGlobal,
      typeText,
      line: name.line - 1,
      col: name.col - 1,
      endLine: lastTypeToken.line - 1,
      endCol: lastTypeToken.col - 1 + lastTypeToken.value.length,
    };
    if (names.has(name.value)) {
      errors.push(moduleError(filePath, name.line, name.col, `name '${name.value}' is declared more than once`));
    } else {
      names.add(name.value);
      declarations.push(declaration);
    }
  }

  const definition: ModuleDefinition = {
    moduleName,
    filePath,
    declarations,
    members: declarations.filter((declaration) => !declaration.isGlobal),
    globals: declarations.filter((declaration) => declaration.isGlobal),
  };
  return { definition, errors };
}

export function loadModuleDefinition(moduleName: string, sourcePath: string): ModuleDefinitionResult {
  const filePath = path.join(path.dirname(sourcePath), `${moduleName}.luard`);
  try {
    return parseModuleDefinition(moduleName, fs.readFileSync(filePath, "utf8"), filePath);
  } catch (error) {
    return { definition: null, errors: [moduleError(filePath, 1, 1, `cannot read module definition: ${errorMessage(error)}`)] };
  }
}

export function importsInDocument(source: string): Array<{ name: string; line: number; col: number }> {
  let tokens: Token[];
  try {
    tokens = new Lexer(source).tokenize();
  } catch {
    return [];
  }
  const imports: Array<{ name: string; line: number; col: number }> = [];
  for (let i = 0; i + 2 < tokens.length; i++) {
    if (tokens[i]!.kind === "import" && tokens[i + 1]!.kind === "Ident" &&
        tokens[i + 1]!.value === "type" && tokens[i + 2]!.kind === "Ident") {
      const token = tokens[i + 2]!;
      imports.push({ name: token.value, line: token.line - 1, col: token.col - 1 });
    }
  }
  return imports;
}

const KEYWORDS = [
  "class", "is", "public", "private", "static", "abstract", "override", "final", "super",
  "operator", "import", "declare", "global", "function", "end", "local", "return", "self",
  "if", "then", "else", "elseif", "while", "for", "do", "repeat", "until", "in", "break",
  "continue", "and", "or", "not", "true", "false", "nil", "const",
];

export function indexDocument(source: string, moduleDefinitions: ModuleDefinition[] = []): DocumentIndex {
  const index: DocumentIndex = { symbols: [], classes: new Map(), members: new Map(), keywords: KEYWORDS };
  let program: Program;
  let tokens: Token[];
  try {
    tokens = new Lexer(source).tokenize();
    program = new Parser(source).parse();
  } catch {
    // Keep completion useful while a document is temporarily incomplete, e.g.
    // immediately after typing `Foo.`. The token-only fallback intentionally
    // indexes declarations and class members without requiring a complete AST.
    try {
      tokens = new Lexer(source).tokenize();
      indexTokens(index, tokens);
      addModuleDefinitions(index, moduleDefinitions);
      return index;
    } catch {
      return index;
    }
  }

  let searchFrom = 0;
  for (const stmt of program.stmts) {
    if (stmt.kind === "ClassDecl") {
      const classToken = findToken(tokens, "class", stmt.line - 1, searchFrom);
      const nameToken = classToken >= 0 ? findToken(tokens, stmt.name, classToken + 1) : -1;
      const endToken = classToken >= 0 ? findMatchingClassEnd(tokens, classToken) : nameToken;
      const symbol = makeSymbol(stmt.name, "class", `class ${stmt.name}${stmt.parent ? ` is ${stmt.parent}` : ""}`, tokens[nameToken] ?? tokens[classToken], tokens[endToken] ?? tokens[nameToken]);
      addSymbol(index, symbol);
      index.classes.set(stmt.name, symbol);

      const members = [...stmt.topLevelMembers, ...stmt.blocks.flatMap((block) => block.members)];
      let memberSearch = Math.max(classToken + 1, 0);
      for (const member of members) {
        const memberName = member.kind === "MethodMember" ? member.name : member.name;
        const memberToken = findMemberToken(tokens, memberName, memberSearch, endToken);
        if (memberToken < 0) continue;
        const memberSymbol = member.kind === "MethodMember"
          ? makeSymbol(member.name, "method", methodSignature(member), tokens[memberToken], tokens[memberToken], stmt.name)
          : makeSymbol(member.name, "field", fieldSignature(member), tokens[memberToken], tokens[memberToken], stmt.name);
        addSymbol(index, memberSymbol);
        index.members.set(`${stmt.name}.${member.name}`, [memberSymbol]);
        memberSearch = memberToken + 1;
      }
      searchFrom = Math.max(endToken + 1, searchFrom);
      continue;
    }

    if (stmt.kind === "ImportDecl") {
      const pos = findToken(tokens, stmt.moduleName, searchFrom);
      if (pos >= 0) addSymbol(index, makeSymbol(stmt.moduleName, "module", `import type ${stmt.moduleName}`, tokens[pos], tokens[pos]));
      searchFrom = Math.max(pos + 1, searchFrom);
    } else if (stmt.kind === "DeclareStmt") {
      const pos = findToken(tokens, stmt.name, searchFrom);
      if (pos >= 0) addSymbol(index, makeSymbol(stmt.name, "variable", `${stmt.isGlobal ? "declare global" : "declare"} ${stmt.name}: ${typeText(stmt.type)}`, tokens[pos], tokens[pos]));
      searchFrom = Math.max(pos + 1, searchFrom);
    }
    collectFunctions(stmt, index, tokens, searchFrom);
  }
  addModuleDefinitions(index, moduleDefinitions);
  return index;
}

function addModuleDefinitions(index: DocumentIndex, definitions: ModuleDefinition[]): void {
  for (const definition of definitions) {
    const moduleSymbol = index.symbols.find((symbol) => symbol.kind === "module" && symbol.name === definition.moduleName);
    if (!moduleSymbol) continue;
    for (const declaration of definition.members) {
      const symbol = makeSymbol(
        declaration.name,
        "field",
        `${declaration.name}: ${declaration.typeText}`,
        { line: declaration.line + 1, col: declaration.col + 1, value: declaration.name } as Token,
        { line: declaration.endLine + 1, col: declaration.endCol + 1, value: declaration.name } as Token,
        definition.moduleName,
      );
      addSymbol(index, symbol);
    }
    for (const declaration of definition.globals) {
      addSymbol(index, makeSymbol(
        declaration.name,
        "variable",
        `declare global ${declaration.name}: ${declaration.typeText}`,
        { line: declaration.line + 1, col: declaration.col + 1, value: declaration.name } as Token,
        { line: declaration.endLine + 1, col: declaration.endCol + 1, value: declaration.name } as Token,
      ));
    }
  }
}

function parseDefinitionType(tokens: Token[], start: number): number {
  let pos = start;
  if (tokens[pos]?.kind === "(") {
    pos++;
    if (tokens[pos]?.kind !== ")") {
      const first = parseDefinitionType(tokens, pos);
      if (first < 0) return -1;
      pos = first;
      while (tokens[pos]?.kind === ",") {
        const next = parseDefinitionType(tokens, pos + 1);
        if (next < 0) return -1;
        pos = next;
      }
    }
    if (tokens[pos]?.kind !== ")") return -1;
    pos++;
    if (tokens[pos]?.kind === "->") {
      const result = parseDefinitionType(tokens, pos + 1);
      if (result < 0) return -1;
      return result;
    }
  } else if (tokens[pos]?.kind === "Ident") {
    pos++;
  } else {
    return -1;
  }
  if (tokens[pos]?.kind === "?") pos++;
  return pos;
}

function nextDeclaration(tokens: Token[], start: number): number {
  for (let i = Math.max(start, 0); i < tokens.length; i++) {
    if (tokens[i]!.kind === "declare" || tokens[i]!.kind === "EOF") return i;
  }
  return tokens.length - 1;
}

function sourceTextBetween(source: string, first: Token, last: Token): string {
  const lines = source.split(/\r?\n/);
  const startLine = lines[first.line - 1] ?? "";
  const endLine = lines[last.line - 1] ?? "";
  if (first.line === last.line) {
    return startLine.slice(first.col - 1, last.col - 1 + last.value.length).trim();
  }
  return [
    startLine.slice(first.col - 1),
    ...lines.slice(first.line, last.line - 1),
    endLine.slice(0, last.col - 1 + last.value.length),
  ].join("\n").trim();
}

function moduleError(filePath: string, line: number, col: number, message: string): ModuleDefinitionError {
  return { message: `${filePath}:${line}: ${message}`, line: Math.max(line - 1, 0), col: Math.max(col - 1, 0) };
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function indexTokens(index: DocumentIndex, tokens: Token[]): DocumentIndex {
  for (let i = 0; i + 1 < tokens.length; i++) {
    const token = tokens[i]!;
    const next = tokens[i + 1]!;

    if (token.kind === "class" && next.kind === "Ident") {
      const end = findMatchingClassEnd(tokens, i);
      const classSymbol = makeSymbol(
        next.value,
        "class",
        `class ${next.value}`,
        next,
        tokens[end] ?? next,
      );
      addSymbol(index, classSymbol);
      index.classes.set(next.value, classSymbol);

      for (let member = i + 1; member < end; member++) {
        const memberToken = tokens[member]!;
        const memberNext = tokens[member + 1];
        if (memberToken.kind === "function" && memberNext) {
          const nameToken = memberNext.kind === "operator" ? tokens[member + 2] : memberNext;
          if (!nameToken) continue;
          const name = memberNext.kind === "operator"
            ? `operator${nameToken.value}`
            : nameToken.value;
          const symbol = makeSymbol(
            name,
            "method",
            `function ${name}(...)`,
            nameToken,
            nameToken,
            next.value,
          );
          addSymbol(index, symbol);
          member++;
          continue;
        }
        if (memberToken.kind === "Ident" && memberNext &&
            (memberNext.kind === "=" || memberNext.kind === ":") &&
            !isParameterToken(tokens, member)) {
          addSymbol(index, makeSymbol(
            memberToken.value,
            "field",
            `field ${memberToken.value}`,
            memberToken,
            memberToken,
            next.value,
          ));
        }
      }
      i = end;
      continue;
    }

    if (token.kind === "import" && next.kind === "Ident" && next.value === "type" && tokens[i + 2]?.kind === "Ident") {
      const module = tokens[i + 2]!;
      addSymbol(index, makeSymbol(module.value, "module", `import type ${module.value}`, module, module));
      i += 2;
    }
    if (token.kind === "declare" && next.kind === "Ident") {
      addSymbol(index, makeSymbol(next.value, "variable", `declare ${next.value}`, next, next));
    }
  }
  return index;
}

function isParameterToken(tokens: Token[], index: number): boolean {
  const previous = tokens[index - 1];
  return previous?.line === tokens[index]!.line &&
    (previous.kind === "(" || previous.kind === ",");
}

function collectFunctions(stmt: Stmt, index: DocumentIndex, tokens: Token[], searchFrom: number): void {
  if (stmt.kind === "Local") {
    stmt.names.forEach((name, i) => {
      const value = stmt.values[i];
      if (value?.kind !== "Function") return;
      const pos = findToken(tokens, name, searchFrom);
      if (pos >= 0) addSymbol(index, makeSymbol(name, "function", functionSignature(name, value.params, value.returnType), tokens[pos], tokens[pos]));
    });
  } else if (stmt.kind === "Assign") {
    stmt.targets.forEach((target, i) => {
      const value = stmt.values[i];
      if (target.kind !== "Ident" || value?.kind !== "Function") return;
      const pos = findToken(tokens, target.name, searchFrom);
      if (pos >= 0) addSymbol(index, makeSymbol(target.name, "function", functionSignature(target.name, value.params, value.returnType), tokens[pos], tokens[pos]));
    });
  }
}

function addSymbol(index: DocumentIndex, symbol: LanguageSymbol): void {
  index.symbols.push(symbol);
  if (!index.members.has(symbol.name)) index.members.set(symbol.name, []);
  index.members.get(symbol.name)!.push(symbol);
}

function makeSymbol(name: string, kind: SymbolKind, signature: string, start?: Token, end?: Token, parent?: string): LanguageSymbol {
  const s = start ?? { line: 1, col: 1 } as Token;
  const e = end ?? s;
  return { name, kind, signature, line: s.line - 1, col: s.col - 1, endLine: e.line - 1, endCol: e.col - 1 + Math.max(name.length, 1), parent };
}

function findToken(tokens: Token[], value: string, from: number, to = tokens.length): number {
  for (let i = Math.max(0, from); i < Math.min(to, tokens.length); i++) if (tokens[i]!.value === value) return i;
  return -1;
}

function findMemberToken(tokens: Token[], name: string, from: number, to: number): number {
  if (name.startsWith("operator")) {
    const operator = name.slice("operator".length);
    for (let i = Math.max(0, from); i + 1 < Math.min(to, tokens.length); i++) {
      if (tokens[i]!.kind === "operator" && tokens[i + 1]!.value === operator) return i;
    }
  }
  return findToken(tokens, name, from, to);
}

function findMatchingClassEnd(tokens: Token[], start: number): number {
  let depth = 0;
  for (let i = start; i < tokens.length; i++) {
    const kind = tokens[i]!.kind;
    if (kind === "class" || kind === "function" || kind === "if" || kind === "while" || kind === "for" || kind === "do" || kind === "repeat" || kind === "public" || kind === "private") {
      depth++;
    } else if (kind === "end") {
      depth--;
      if (depth === 0) return i;
    } else if (kind === "until") {
      depth--;
    }
  }
  return start;
}

function typeText(type: TypeExpr | null): string { return type ? type.kind === "TypeOptional" ? `${typeText(type.inner)}?` : type.kind === "TypeTuple" ? `(${type.types.map(typeText).join(", ")})` : type.name : "any"; }
function paramText(param: Param): string { return param.kind === "Vararg" ? "..." : `${param.name}${param.type ? `: ${typeText(param.type)}` : ""}`; }
function functionSignature(name: string, params: Param[], returnType: TypeExpr | null): string { return `function ${name}(${params.map(paramText).join(", ")})${returnType ? `: ${typeText(returnType)}` : ""}`; }
function methodSignature(method: MethodMember): string { return `${method.isStatic ? "static " : ""}${functionSignature(method.name, method.params, method.returnType)}${method.isAbstract ? " abstract" : ""}`; }
function fieldSignature(member: Extract<Member, { kind: "FieldMember" }>): string { return `${member.name}${member.type ? `: ${typeText(member.type)}` : ""}`; }
