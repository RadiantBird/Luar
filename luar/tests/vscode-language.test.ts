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

  it("skips declare class blocks without reporting syntax errors", () => {
    const source = "declare class Dog\n    name: string\n    function bark(times: number): string\n    static function create(): Dog\nend\ndeclare dog: Dog\n";
    const result = parseModuleDefinition("clsdef", source);

    expect(result.errors).toEqual([]);
    expect(result.definition?.members.map((member) => member.name)).toEqual(["dog"]);

    const unclosed = parseModuleDefinition("clsdef", "declare class Dog\n    name: string\n");
    expect(unclosed.errors[0]?.message).toContain("expected 'end'");
  });

  it("skips class-syntax declare class blocks with bodies and friends", () => {
    const source = "declare class Part is\n    friend class Instance\n    public is\n        Anchored = false\n    end\n    private is\n        static function new()\n            if true then\n                return 1\n            end\n        end\n    end\nend\ndeclare version: string\n";
    const result = parseModuleDefinition("Part", source);

    expect(result.errors).toEqual([]);
    expect(result.definition?.members.map((member) => member.name)).toEqual(["version"]);
  });

  it("skips declare class blocks whose methods have no body", () => {
    const source = [
      "declare class Part is",
      "    friend class Instance",
      "    public is",
      "        Anchored = false",
      "    end",
      "    private is",
      "        static function new(): Part",
      "    end",
      "end",
      "",
      "declare class Instance is",
      "    public is",
      "        static function new(name: string): Part?",
      "    end",
      "end",
    ].join("\n");
    const result = parseModuleDefinition("Part", source);

    expect(result.errors).toEqual([]);
    expect(result.definition?.members).toEqual([]);
  });

  it("indexes module members while the source is incomplete after a dot", () => {
    const definition = parseModuleDefinition("qaz", "declare wsx: string\n").definition!;
    const index = indexDocument("import type qaz\nqaz.", [definition]);

    expect(index.symbols.some((symbol) => symbol.parent === "qaz" && symbol.name === "wsx")).toBe(true);
  });

  it("does not recognize legacy imports as module definitions", () => {
    expect(indexDocument("import qaz\nqaz.").symbols.some((symbol) => symbol.kind === "module")).toBe(false);
  });
  it("accepts type declarations, template headers and declare function", () => {
    const source = [
      "type Arithmetic = Vector3 | number",
      "",
      "template <T>",
      "export type MyTable = { id: number, ref: T }",
      "",
      "template <T>",
      "declare function add(a: T, b: T): number",
      "declare global function log(message: string, ...)",
      "declare list: { number }",
      "declare pair: MyTable<number>",
      "declare maybe: (number | string)?",
      "declare run: (string, ...number) -> boolean | nil",
      "declare version: string",
      "",
    ].join("\n");
    const result = parseModuleDefinition("m", source);

    expect(result.errors).toEqual([]);
    expect(result.definition?.members.map((member) => member.name)).toEqual(["add", "list", "pair", "maybe", "run", "version"]);
    expect(result.definition?.globals.map((member) => member.name)).toEqual(["log"]);
    expect(result.definition?.members[0]).toMatchObject({ typeText: "(a: T, b: T): number" });
  });

  it("accepts classes with generic and operator methods after a type declaration", () => {
    const source = [
      "type Arithmetic = Vector3 | number",
      "",
      "declare class Vector3 is",
      "    public is",
      "        x: number",
      "        function length(v: Vector3): number",
      "        template <A> function scale(v: Vector3, k: A): Vector3",
      "        function operator*(a: Vector3, b: Arithmetic): Vector3",
      "        function operator/(a: Vector3, b: Arithmetic): Vector3",
      "    end",
      "end",
      "",
      "declare class Instance is",
      "    public is",
      "        template <T> function GetChildren(Inst: Instance): { T }",
      "        function Destroy(Inst: Instance): ()",
      "        static function new(classname: string): Instance?",
      "    end",
      "end",
      "",
    ].join("\n");
    const result = parseModuleDefinition("RCBN", source);

    expect(result.errors).toEqual([]);
  });

  it("explains that declared classes are already global", () => {
    const result = parseModuleDefinition("m", "declare global class Instance is\n    public is\n    end\nend\ndeclare version: string\n");

    expect(result.errors[0]?.message).toContain("a declared class is already global");
  });

  it("reports a malformed type alias and still reads the rest", () => {
    const result = parseModuleDefinition("m", "type Broken = \ndeclare version: string\n");

    expect(result.errors[0]?.message).toContain("expected type");
    expect(result.definition?.members.map((member) => member.name)).toEqual(["version"]);
  });
});
