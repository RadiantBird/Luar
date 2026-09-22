import { describe, it, expect } from "vitest";
import { Parser } from "../src/parser/parser.js";
import type { ClassDecl, MethodMember, FieldMember, Stmt } from "../src/parser/ast.js";

function parse(src: string) {
  return new Parser(src).parse();
}

function classDecl(src: string): ClassDecl {
  const prog = parse(src);
  const decl = prog.stmts[0];
  if (decl?.kind !== "ClassDecl") throw new Error("not a ClassDecl");
  return decl;
}

describe("Parser: クラス宣言", () => {
  it("基本的なクラス宣言", () => {
    const c = classDecl("class Foo is\nend");
    expect(c.name).toBe("Foo");
    expect(c.isAbstract).toBe(false);
    expect(c.parent).toBeNull();
  });

  it("abstractクラス", () => {
    const c = classDecl("class Foo is abstract\nend");
    expect(c.isAbstract).toBe(true);
  });

  it("継承", () => {
    const c = classDecl("class Foo is Bar\nend");
    expect(c.parent).toBe("Bar");
  });

  it("public/privateブロック", () => {
    const src = `class Foo is
      public is
        function greet() end
      end
      private is
        x = 0
      end
    end`;
    const c = classDecl(src);
    expect(c.blocks).toHaveLength(2);
    expect(c.blocks[0]!.access).toBe("public");
    expect(c.blocks[1]!.access).toBe("private");
  });

  it("staticメソッド", () => {
    const src = `class Foo is
      public is
        static function new() end
      end
    end`;
    const c = classDecl(src);
    const method = c.blocks[0]!.members[0] as MethodMember;
    expect(method.kind).toBe("MethodMember");
    expect(method.isStatic).toBe(true);
    expect(method.name).toBe("new");
  });

  it("abstractメソッド（bodyがnull）", () => {
    const src = `class Foo is abstract
      public is
        function explain() abstract
      end
    end`;
    const c = classDecl(src);
    const method = c.blocks[0]!.members[0] as MethodMember;
    expect(method.isAbstract).toBe(true);
    expect(method.body).toBeNull();
  });

  it("override / final", () => {
    const src = `class Foo is
      public is
        function run() override final end
      end
    end`;
    const c = classDecl(src);
    const method = c.blocks[0]!.members[0] as MethodMember;
    expect(method.isOverride).toBe(true);
    expect(method.isFinal).toBe(true);
  });

  it("operatorオーバーロード", () => {
    const src = `class Foo is
      public is
        function operator==(other: Foo)
          return false
        end
      end
    end`;
    const c = classDecl(src);
    const method = c.blocks[0]!.members[0] as MethodMember;
    expect(method.isOperator).toBe(true);
    expect(method.operatorOp).toBe("==");
    expect(method.name).toBe("operator==");
  });

  it("フィールド宣言", () => {
    const src = `class Foo is
      private is
        Year = 0
      end
    end`;
    const c = classDecl(src);
    const field = c.blocks[0]!.members[0] as FieldMember;
    expect(field.kind).toBe("FieldMember");
    expect(field.name).toBe("Year");
    expect(field.value).toMatchObject({ kind: "Number", value: "0" });
  });

  it("型アノテーション付きパラメータ", () => {
    const src = `class Foo is
      public is
        static function new(year: number) end
      end
    end`;
    const c = classDecl(src);
    const method = c.blocks[0]!.members[0] as MethodMember;
    expect(method.params).toHaveLength(1);
    expect(method.params[0]).toMatchObject({ kind: "Param", name: "year" });
  });
});

describe("Parser: 通常文", () => {
  it("local宣言", () => {
    const prog = parse("local x = 42");
    expect(prog.stmts[0]).toMatchObject({ kind: "Local", names: ["x"] });
  });

  it("代入文", () => {
    const prog = parse("x = 10");
    expect(prog.stmts[0]).toMatchObject({ kind: "Assign" });
  });

  it("if/then/else", () => {
    const prog = parse("if x == 1 then local y = 2 else local z = 3 end");
    expect(prog.stmts[0]).toMatchObject({ kind: "If" });
  });

  it("whileループ", () => {
    const prog = parse("while true do end");
    expect(prog.stmts[0]).toMatchObject({ kind: "While" });
  });

  it("numericFor", () => {
    const prog = parse("for i = 1, 10 do end");
    expect(prog.stmts[0]).toMatchObject({ kind: "NumericFor", name: "i" });
  });

  it("return文", () => {
    const prog = parse("return false");
    const ret = prog.stmts[0] as Extract<Stmt, { kind: "Return" }>;
    expect(ret.kind).toBe("Return");
    expect(ret.values[0]).toMatchObject({ kind: "False" });
  });

  it("テーブルコンストラクタ", () => {
    const prog = parse("local t = {1, 2, x = 3}");
    const local = prog.stmts[0] as Extract<Stmt, { kind: "Local" }>;
    expect(local.values[0]).toMatchObject({ kind: "Table" });
  });
});

describe("Parser: LPL.mdサンプル全体", () => {
  it("hello.luarのクラス宣言をパース", () => {
    const src = `
class Lua is
    private is
        Year = 0
        function explode()
            print("Boom!!")
        end
    end
    public is
        static function new(year: number)
            self.Year = year
        end
        function greet()
            print(\`Hello from {self.Year}!\`)
        end
        function operator==(AnotherRock: Lua)
            return false
        end
    end
end
local L = Lua.new(1993)
L.greet()
`;
    const prog = parse(src);
    expect(prog.stmts).toHaveLength(3); // ClassDecl, Local, ExprStmt
    const c = prog.stmts[0] as ClassDecl;
    expect(c.kind).toBe("ClassDecl");
    expect(c.name).toBe("Lua");
    expect(c.blocks).toHaveLength(2);
  });
});
