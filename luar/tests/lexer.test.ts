import { describe, it, expect } from "vitest";
import { Lexer } from "../src/lexer/lexer.js";
import type { TokenKind } from "../src/lexer/token.js";

function kinds(src: string): TokenKind[] {
  return new Lexer(src).tokenize().map((t) => t.kind);
}

function values(src: string): string[] {
  return new Lexer(src).tokenize().filter(t => t.kind !== "EOF").map((t) => t.value);
}

describe("Lexer", () => {
  it("gotoとlabel区切りをトークン化する", () => {
    expect(kinds("goto exit\n::exit::")).toEqual(["goto", "Ident", ":", ":", "Ident", ":", ":", "EOF"]);
  });

  it("クラス宣言の基本トークン", () => {
    expect(kinds("class Lua is")).toEqual(["class", "Ident", "is", "EOF"]);
  });

  it("public / private キーワード", () => {
    expect(kinds("public private static")).toEqual(["public", "private", "static", "EOF"]);
  });

  it("abstract / override / final / super キーワード", () => {
    expect(kinds("abstract override final super")).toEqual([
      "abstract", "override", "final", "super", "EOF",
    ]);
  });

  it("operator キーワード", () => {
    expect(kinds("operator==")).toEqual(["operator", "==", "EOF"]);
  });

  it("数値リテラル（整数・小数・16進数）", () => {
    expect(kinds("42 3.14 0xFF")).toEqual(["Number", "Number", "Number", "EOF"]);
    expect(values("42 3.14 0xFF")).toEqual(["42", "3.14", "0xFF"]);
  });

  it("文字列リテラル（ダブルクォート・シングルクォート）", () => {
    expect(kinds('"hello" \'world\'')).toEqual(["String", "String", "EOF"]);
    expect(values('"hello" \'world\'')).toEqual(["hello", "world"]);
  });

  it("バッククォートテンプレート文字列", () => {
    const toks = new Lexer("`Hello from {self.Year}!`").tokenize();
    expect(toks[0].kind).toBe("String");
    expect(toks[0].value).toBe("`Hello from {self.Year}!`");
  });

  it("一行コメントをスキップ", () => {
    expect(kinds("local x -- this is a comment\nlocal y")).toEqual([
      "local", "Ident", "local", "Ident", "EOF",
    ]);
  });

  it("ブロックコメントをスキップ", () => {
    expect(kinds("local x --[[ block comment ]] local y")).toEqual([
      "local", "Ident", "local", "Ident", "EOF",
    ]);
  });

  it("2文字演算子", () => {
    expect(kinds("== ~= <= >= .. ...")).toEqual([
      "==", "~=", "<=", ">=", "..", "...", "EOF",
    ]);
  });

  it("行番号・列番号の追跡", () => {
    const toks = new Lexer("class\n  Lua").tokenize();
    expect(toks[0]).toMatchObject({ kind: "class", line: 1, col: 1 });
    expect(toks[1]).toMatchObject({ kind: "Ident", value: "Lua", line: 2, col: 3 });
  });

  it("LPL.mdサンプル: クラス宣言ヘッダー", () => {
    const src = `class Lua is
    private is
        Year = 0
    end`;
    const toks = new Lexer(src).tokenize().filter(t => t.kind !== "EOF");
    expect(toks[0].kind).toBe("class");
    expect(toks[1]).toMatchObject({ kind: "Ident", value: "Lua" });
    expect(toks[2].kind).toBe("is");
    expect(toks[3].kind).toBe("private");
    expect(toks[4].kind).toBe("is");
  });

  it("不正な文字でLexErrorを投げる", () => {
    expect(() => new Lexer("local x = @").tokenize()).toThrow("unexpected character");
  });
});
