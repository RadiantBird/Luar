import { describe, it, expect } from "vitest";
import { Parser } from "../src/parser/parser.js";
import { Codegen } from "../src/codegen/codegen.js";

function gen(src: string): string {
  const prog = new Parser(src).parse();
  return new Codegen().generate(prog).trim();
}

function hasLine(src: string, expected: string) {
  const output = gen(src);
  const lines = output.split("\n").map(l => l.trim());
  expect(lines).toContain(expected.trim());
}

describe("Codegen: クラステーブル", () => {
  it("親なしクラス — テーブルと__index", () => {
    hasLine(`class Foo is end`, "local Foo = {}");
    hasLine(`class Foo is end`, "Foo.__index = Foo");
  });

  it("継承クラス — setmetatableで親を参照", () => {
    const src = `
      class Base is end
      class Child is Base end
    `;
    hasLine(src, "local Child = setmetatable({}, { __index = Base })");
    hasLine(src, "Child.__index = Child");
  });
});

describe("Codegen: コンストラクタ (new)", () => {
  it("newがフィールドの初期値を自動生成する", () => {
    const src = `
      class Foo is
        private is
          Count = 0
          Name = "hello"
        end
        public is
          static function new() end
        end
      end
    `;
    const output = gen(src);
    expect(output).toContain("local self = setmetatable({}, Foo)");
    expect(output).toContain('self.Count = 0');
    expect(output).toContain('self.Name = "hello"');
    expect(output).toContain("return self");
  });

  it("コンストラクタのパラメータがそのまま出力される", () => {
    const src = `
      class Foo is
        public is
          static function new(x: number) end
        end
      end
    `;
    hasLine(src, "function Foo.new(x)");
  });
});

describe("Codegen: メソッド種別", () => {
  it("publicインスタンスメソッドはselfを第1引数に追加", () => {
    const src = `
      class Foo is
        public is
          function greet() end
        end
      end
    `;
    hasLine(src, "function Foo.greet(self)");
  });

  it("publicスタティックメソッドはselfなし", () => {
    const src = `
      class Foo is
        public is
          static function panic() end
        end
      end
    `;
    hasLine(src, "function Foo.panic()");
  });

  it("privateメソッドはlocal functionに変換", () => {
    const src = `
      class Foo is
        private is
          function explode() end
        end
        public is
          static function new() end
        end
      end
    `;
    hasLine(src, "local function explode(self)");
  });

  it("デストラクタ (free) はselfを第1引数に追加", () => {
    const src = `
      class Foo is
        public is
          static function new() end
          function free() end
        end
      end
    `;
    hasLine(src, "function Foo.free(self)");
  });
});

describe("Codegen: operatorオーバーロード", () => {
  it("operator== → __eq メタメソッド", () => {
    const src = `
      class Foo is
        public is
          function operator==(other: Foo)
            return false
          end
        end
      end
    `;
    hasLine(src, "Foo.__eq = function(self, other)");
  });

  it("operator< → __lt", () => {
    const src = `
      class Foo is
        public is
          function operator<(other: Foo)
            return true
          end
        end
      end
    `;
    hasLine(src, "Foo.__lt = function(self, other)");
  });
});

describe("Codegen: LPL.mdサンプル全体", () => {
  it("hello.luarの変換結果が期待する構造を持つ", () => {
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
local L2 = Lua.new(2006)
L.greet()
`;
    const output = gen(src);

    // Class table
    expect(output).toContain("local Lua = {}");
    expect(output).toContain("Lua.__index = Lua");

    // Operator metamethod
    expect(output).toContain("Lua.__eq = function(self, AnotherRock)");
    expect(output).toContain("return false");

    // Private method as local function
    expect(output).toContain('local function explode(self)');
    expect(output).toContain('"Boom!!"');

    // Constructor
    expect(output).toContain("function Lua.new(year)");
    expect(output).toContain("local self = setmetatable({}, Lua)");
    expect(output).toContain("self.Year = 0");
    expect(output).toContain("return self");

    // Instance method with self
    expect(output).toContain("function Lua.greet(self)");

    // Template string is preserved
    expect(output).toContain("`Hello from {self.Year}!`");

    // Non-class statements pass through
    expect(output).toContain("local L = Lua.new(1993)");
    expect(output).toContain("local L2 = Lua.new(2006)");
    // dot→colon変換: Luar の L.greet() → Luau の L:greet()
    expect(output).toContain("L:greet()");
  });
});

describe("Codegen: dot→colon変換", () => {
  it("インスタンスメソッド呼び出しを:に変換", () => {
    const src = `
      class Foo is
        public is
          static function new() end
          function greet() end
        end
      end
      local f = Foo.new()
      f.greet()
    `;
    const output = gen(src);
    expect(output).toContain("f:greet()");
  });

  it("staticメソッド呼び出しは.のまま", () => {
    const src = `
      class Foo is
        public is
          static function new() end
          static function panic() end
        end
      end
      local f = Foo.new()
      Foo.panic()
    `;
    const output = gen(src);
    expect(output).toContain("Foo.panic()");
    expect(output).not.toContain("Foo:panic()");
  });

  it("self.method()もself:method()に変換", () => {
    const src = `
      class Foo is
        public is
          static function new() end
          function greet() end
          function run()
            self.greet()
          end
        end
      end
    `;
    const output = gen(src);
    expect(output).toContain("self:greet()");
  });

  it("型が不明な変数の.呼び出しはそのまま", () => {
    // x の型が不明なのでドットのまま（安全なフォールバック）
    const src = `x.doSomething()`;
    const output = gen(src);
    expect(output).toContain("x.doSomething()");
  });
});

describe("Codegen: new自動継承", () => {
  it("子クラスにnewがない場合、親のnewを継承したnewを生成する", () => {
    const src = `
      class Base is
        public is
          static function new(x: number) end
        end
      end
      class Child is Base
        public is
          function greet() end
        end
      end
    `;
    const output = gen(src);
    expect(output).toContain("function Child.new(x)");
    expect(output).toContain("local self = Base.new(x)");
    expect(output).toContain("setmetatable(self, Child)");
    expect(output).toContain("return self");
  });

  it("子クラス固有のフィールドもinheritedなnewで初期化される", () => {
    const src = `
      class Base is
        public is
          static function new() end
        end
      end
      class Child is Base
        private is
          Count = 0
        end
        public is
          function greet() end
        end
      end
    `;
    const output = gen(src);
    expect(output).toContain("function Child.new()");
    expect(output).toContain("self.Count = 0");
  });

  it("多段継承でもnewを見つけて継承する", () => {
    const src = `
      class A is
        public is
          static function new(n: number) end
        end
      end
      class B is A end
      class C is B end
    `;
    const output = gen(src);
    // B: inherits from A
    expect(output).toContain("function B.new(n)");
    expect(output).toContain("local self = A.new(n)");
    // C: inherits from B (which inherited from A)
    expect(output).toContain("function C.new(n)");
    expect(output).toContain("local self = B.new(n)");
  });

  it("祖先にnewがない場合はnewを生成しない", () => {
    const src = `
      class Base is end
      class Child is Base end
    `;
    const output = gen(src);
    expect(output).not.toContain("Child.new");
  });

  it("子クラスが自分でnewを定義している場合はそちらが優先される", () => {
    const src = `
      class Base is
        public is
          static function new(x: number) end
        end
      end
      class Child is Base
        public is
          static function new(y: string) end
        end
      end
    `;
    const output = gen(src);
    expect(output).toContain("function Child.new(y)");
    expect(output).not.toContain("function Child.new(x)");
  });
});

describe("Codegen: super変換", () => {
  it("super.method() → ParentName.method(self)", () => {
    const src = `
      class Base is
        public is
          function explain() end
        end
      end
      class Child is Base
        public is
          function explain() override
            super.explain()
          end
        end
      end
    `;
    hasLine(src, "Base.explain(self)");
  });

  it("super.method(args) → ParentName.method(self, args)", () => {
    const src = `
      class Base is
        public is
          function greet(name: string) end
        end
      end
      class Child is Base
        public is
          function greet(name: string) override
            super.greet(name)
          end
        end
      end
    `;
    hasLine(src, "Base.greet(self, name)");
  });

  it("LPL.mdのC言語例: super変換", () => {
    const src = `
      class ASM is
        public is
          function explain() end
        end
      end
      class C is ASM
        public is
          function explain() override
            super.explain()
            print("made with English")
          end
        end
      end
    `;
    const output = gen(src);
    expect(output).toContain("ASM.explain(self)");
  });
});

describe("Codegen: 通常文・式", () => {
  it("if/then/else", () => {
    const src = `if x == 1 then local y = 2 else local z = 3 end`;
    const output = gen(src);
    expect(output).toContain("if x == 1 then");
    expect(output).toContain("else");
    expect(output).toContain("end");
  });

  it("whileループ", () => {
    hasLine(`while true do end`, "while true do");
  });

  it("numericFor", () => {
    hasLine(`for i = 1, 10 do end`, "for i = 1, 10 do");
  });

  it("テーブルコンストラクタ", () => {
    hasLine(`local t = {1, 2, x = 3}`, "local t = { 1, 2, x = 3 }");
  });
});
