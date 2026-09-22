import { describe, expect, it } from "vitest";
import { indexDocument, normalizeIncludeMacros, parseModuleDefinition } from "../../luar-vscode/src/language";

describe("VS Code module definitions", () => {
  it("goto labelをアウトラインと補完へ載せる", () => {
    const index = indexDocument("goto exit\n::exit::");
    expect(index.keywords).toContain("goto");
    expect(index.symbols).toEqual(expect.arrayContaining([
      expect.objectContaining({ kind: "label", name: "exit", signature: "::exit::" }),
    ]));
  });

  it("normalizes declaration-form include macros for editor parsing", () => {
    expect(normalizeIncludeMacros('local mod = !include("@./mod.luar") -- module source')).toBe("local mod = {} -- module source");
    expect(normalizeIncludeMacros("const mod = !include('./mod.luar')")).toBe("const mod = {}");
  });

  it("parses declarations and preserves type text and positions", () => {
    const result = parseModuleDefinition("qaz", "-- comment\ndeclare wsx: (string, number)?\ndeclare run: () -> ()\ndeclare transform: (string?, number) -> (string, number)\ndeclare global workspace: workspace\n", "qaz.luard");

    expect(result.errors).toEqual([]);
    expect(result.definition?.members[0]).toMatchObject({
      name: "wsx",
      typeText: "(string, number)?",
      line: 1,
      col: 8,
    });
    expect(result.definition?.members[1]).toMatchObject({ name: "run", typeText: "() -> ()" });
    expect(result.definition?.members[2]).toMatchObject({ name: "transform", typeText: "(string?, number) -> (string, number)" });
    expect(result.definition?.globals[0]?.name).toBe("workspace");
  });

  it("rejects malformed and duplicate declarations", () => {
    const result = parseModuleDefinition("qaz", "declare value: string\ndeclare value: number\ndeclare broken\ndeclare run: () ->\n");

    expect(result.errors).toHaveLength(3);
    expect(result.errors[0]?.message).toContain("declared more than once");
    expect(result.errors[1]?.message).toContain("expected ':'");
    expect(result.errors[2]?.message).toContain("expected type");
  });

  it("indexes module members while the source is incomplete after a dot", () => {
    const definition = parseModuleDefinition("qaz", "declare wsx: string\n").definition!;
    const index = indexDocument("import type qaz\nqaz.", [definition]);

    expect(index.symbols.some((symbol) => symbol.parent === "qaz" && symbol.name === "wsx")).toBe(true);
  });

  it("does not recognize legacy imports as module definitions", () => {
    expect(indexDocument("import qaz\nqaz.").symbols.some((symbol) => symbol.kind === "module")).toBe(false);
  });
});
