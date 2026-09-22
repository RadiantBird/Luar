export type TokenKind =
  // Luar keywords
  | "class" | "is" | "public" | "private" | "static"
  | "abstract" | "override" | "final" | "super"
  | "operator" | "import" | "declare" | "global"
  // Lua/Luau keywords
  | "function" | "end" | "local" | "return" | "self"
  | "if" | "then" | "else" | "elseif"
  | "while" | "for" | "do" | "repeat" | "until" | "in"
  | "break" | "continue" | "goto"
  | "and" | "or" | "not"
  | "true" | "false" | "nil"
  // Literals
  | "Number" | "String" | "Ident"
  // Delimiters
  | "(" | ")" | "{" | "}" | "[" | "]"
  // Punctuation
  | "." | "," | ":" | ":=" | ";" | "->"
  // Assignment & comparison
  | "=" | "==" | "~="
  | "<" | ">" | "<=" | ">="
  // Arithmetic
  | "+" | "-" | "*" | "/" | "//" | "%" | "^" | "#"
  // String concat & vararg
  | ".." | "..."
  // Optional type marker
  | "?"
  // Special
  | "EOF";

export interface Token {
  kind: TokenKind;
  value: string;
  line: number;
  col: number;
}

export const KEYWORDS: ReadonlySet<TokenKind> = new Set<TokenKind>([
  "class", "is", "public", "private", "static",
  "abstract", "override", "final", "super",
  "operator", "import", "declare", "global",
  "function", "end", "local", "return", "self",
  "if", "then", "else", "elseif",
  "while", "for", "do", "repeat", "until", "in",
  "break", "continue", "goto",
  "and", "or", "not",
  "true", "false", "nil",
]);
