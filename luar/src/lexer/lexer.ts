import { type Token, type TokenKind, KEYWORDS } from "./token.js";

export class LexError extends Error {
  constructor(msg: string, public line: number, public col: number) {
    super(`[${line}:${col}] ${msg}`);
    this.name = "LexError";
  }
}

export class Lexer {
  private pos = 0;
  private line = 1;
  private col = 1;

  constructor(private src: string) {}

  tokenize(): Token[] {
    const tokens: Token[] = [];
    while (true) {
      const tok = this.next();
      tokens.push(tok);
      if (tok.kind === "EOF") break;
    }
    return tokens;
  }

  private peek(offset = 0): string {
    return this.src[this.pos + offset] ?? "";
  }

  private advance(): string {
    const ch = this.src[this.pos++] ?? "";
    if (ch === "\n") { this.line++; this.col = 1; }
    else { this.col++; }
    return ch;
  }

  private next(): Token {
    this.skipWhitespaceAndComments();

    const line = this.line;
    const col = this.col;

    if (this.pos >= this.src.length) {
      return { kind: "EOF", value: "", line, col };
    }

    const ch = this.peek();

    if (isDigit(ch)) return this.readNumber(line, col);
    if (ch === '"' || ch === "'") return this.readString(ch, line, col);
    if (ch === "`") return this.readTemplateString(line, col);
    if (isIdentStart(ch)) return this.readIdent(line, col);

    return this.readSymbol(line, col);
  }

  private skipWhitespaceAndComments(): void {
    while (this.pos < this.src.length) {
      const ch = this.peek();

      if (ch === " " || ch === "\t" || ch === "\r" || ch === "\n") {
        this.advance();
        continue;
      }

      // comment
      if (ch === "-" && this.peek(1) === "-") {
        this.advance(); this.advance();
        // long comment --[[...]]
        if (this.peek() === "[" && this.peek(1) === "[") {
          this.advance(); this.advance();
          this.skipLongString();
        } else {
          while (this.pos < this.src.length && this.peek() !== "\n") this.advance();
        }
        continue;
      }

      break;
    }
  }

  private skipLongString(): void {
    while (this.pos < this.src.length) {
      if (this.peek() === "]" && this.peek(1) === "]") {
        this.advance(); this.advance();
        return;
      }
      this.advance();
    }
    throw new LexError("unterminated long string", this.line, this.col);
  }

  private readNumber(line: number, col: number): Token {
    let value = "";
    // hex
    if (this.peek() === "0" && (this.peek(1) === "x" || this.peek(1) === "X")) {
      value += this.advance() + this.advance();
      while (isHexDigit(this.peek())) value += this.advance();
    } else {
      while (isDigit(this.peek())) value += this.advance();
      if (this.peek() === "." && isDigit(this.peek(1))) {
        value += this.advance();
        while (isDigit(this.peek())) value += this.advance();
      }
      if (this.peek() === "e" || this.peek() === "E") {
        value += this.advance();
        if (this.peek() === "+" || this.peek() === "-") value += this.advance();
        while (isDigit(this.peek())) value += this.advance();
      }
    }
    return { kind: "Number", value, line, col };
  }

  private readString(quote: string, line: number, col: number): Token {
    this.advance(); // opening quote
    let value = "";
    while (this.pos < this.src.length) {
      const ch = this.advance();
      if (ch === quote) return { kind: "String", value, line, col };
      if (ch === "\n") throw new LexError("unterminated string", line, col);
      if (ch === "\\") value += "\\" + this.advance();
      else value += ch;
    }
    throw new LexError("unterminated string", line, col);
  }

  // Luau backtick template strings `...{expr}...`
  private readTemplateString(line: number, col: number): Token {
    this.advance(); // opening `
    let value = "`";
    let depth = 0;
    while (this.pos < this.src.length) {
      const ch = this.peek();
      if (ch === "`" && depth === 0) {
        value += this.advance();
        return { kind: "String", value, line, col };
      }
      if (ch === "{") depth++;
      if (ch === "}") depth--;
      value += this.advance();
    }
    throw new LexError("unterminated template string", line, col);
  }

  private readIdent(line: number, col: number): Token {
    let value = "";
    while (isIdentPart(this.peek())) value += this.advance();
    const kind: TokenKind = (KEYWORDS as ReadonlySet<string>).has(value)
      ? (value as TokenKind)
      : "Ident";
    return { kind, value, line, col };
  }

  private readSymbol(line: number, col: number): Token {
    const ch = this.advance();
    const next = this.peek();

    switch (ch) {
      case "(": return tok("(", ch, line, col);
      case ")": return tok(")", ch, line, col);
      case "{": return tok("{", ch, line, col);
      case "}": return tok("}", ch, line, col);
      case "[": return tok("[", ch, line, col);
      case "]": return tok("]", ch, line, col);
      case ",": return tok(",", ch, line, col);
      case ";": return tok(";", ch, line, col);
      case "#": return tok("#", ch, line, col);
      case "^": return tok("^", ch, line, col);
      case "%": return tok("%", ch, line, col);
      case "+": return tok("+", ch, line, col);
      case "*": return tok("*", ch, line, col);
      case "&": return tok("&" as TokenKind, ch, line, col);
      case "|": return tok("|" as TokenKind, ch, line, col);
      case "~":
        if (next === "=") { this.advance(); return tok("~=", "~=", line, col); }
        return tok("~" as TokenKind, ch, line, col);
      case "=":
        if (next === "=") { this.advance(); return tok("==", "==", line, col); }
        return tok("=", ch, line, col);
      case "<":
        if (next === "=") { this.advance(); return tok("<=", "<=", line, col); }
        return tok("<", ch, line, col);
      case ">":
        if (next === "=") { this.advance(); return tok(">=", ">=", line, col); }
        return tok(">", ch, line, col);
      case "-":
        if (next === ">") { this.advance(); return tok("->", "->", line, col); }
        return tok("-", ch, line, col);
      case "/":
        if (next === "/") { this.advance(); return tok("//", "//", line, col); }
        return tok("/", ch, line, col);
      case ".":
        if (next === "." && this.peek(1) === ".") {
          this.advance(); this.advance(); return tok("...", "...", line, col);
        }
        if (next === ".") { this.advance(); return tok("..", "..", line, col); }
        return tok(".", ch, line, col);
      case ":":
        if (next === "=") { this.advance(); return tok(":=", ":=", line, col); }
        return tok(":", ch, line, col);
      case "?": return tok("?", ch, line, col);
    }

    throw new LexError(`unexpected character: ${JSON.stringify(ch)}`, line, col);
  }
}

function tok(kind: TokenKind, value: string, line: number, col: number): Token {
  return { kind, value, line, col };
}

function isDigit(ch: string): boolean { return ch >= "0" && ch <= "9"; }
function isHexDigit(ch: string): boolean {
  return isDigit(ch) || (ch >= "a" && ch <= "f") || (ch >= "A" && ch <= "F");
}
function isIdentStart(ch: string): boolean {
  return (ch >= "a" && ch <= "z") || (ch >= "A" && ch <= "Z") || ch === "_";
}
function isIdentPart(ch: string): boolean {
  return isIdentStart(ch) || isDigit(ch);
}
