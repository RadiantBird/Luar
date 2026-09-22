import { describe, it, expect } from "vitest";
import { Parser } from "../src/parser/parser.js";
import { Checker } from "../src/checker/checker.js";

function check(src: string) {
  const prog = new Parser(src).parse();
  return new Checker().check(prog);
}

function messages(src: string): string[] {
  return check(src).map((e) => e.message);
}

function ok(src: string) {
  expect(check(src)).toHaveLength(0);
}

function err(src: string, contains: string) {
  const errs = messages(src);
  expect(errs.some((m) => m.includes(contains))).toBe(true);
}

// ─── 正常ケース ───────────────────────────────────────────────────────────────

describe("Checker: 正常ケース", () => {
  it("シンプルなクラス", () => ok(`
    class Foo is
      public is
        static function new() end
        function greet() end
      end
    end
  `));

  it("abstractクラスにabstractメソッド", () => ok(`
    class Foo is abstract
      public is
        function explain() abstract
      end
    end
  `));

  it("継承 + override", () => ok(`
    class Base is abstract
      public is
        function explain() abstract
      end
    end
    class Child is Base
      public is
        function explain() override end
      end
    end
  `));

  it("多段継承 + override", () => ok(`
    class A is abstract
      public is
        function run() abstract
      end
    end
    class B is A
      public is
        function run() override end
      end
    end
    class C is B
      public is
        function run() override end
      end
    end
  `));

  it("finalメソッドは同クラスで定義できる", () => ok(`
    class Foo is
      public is
        function build() final end
      end
    end
  `));

  it("operatorオーバーロード（non-static）", () => ok(`
    class Foo is
      public is
        function operator==(other: Foo)
          return false
        end
      end
    end
  `));

  it("LPL.mdのC言語クラス継承例", () => ok(`
    class Language is abstract
      public is
        function explain() abstract
      end
    end
    class Binary is Language
      public is
        function explain() override end
      end
    end
    class ASM is Binary
      public is
        function explain() override end
        function assembly() final end
      end
    end
    class C is ASM
      public is
        function explain() override end
      end
    end
  `));
});

// ─── エラーケース ─────────────────────────────────────────────────────────────

describe("Checker: abstractルール", () => {
  it("non-abstractクラスにabstractメソッド", () => err(`
    class Foo is
      public is
        function explain() abstract
      end
    end
  `, "not abstract"));

  it("abstractのないクラスでabstractを使うとエラー", () => err(`
    class Foo is
      public is
        function run() abstract
      end
    end
  `, "not abstract"));
});

describe("Checker: overrideルール", () => {
  it("親クラスなしでoverride", () => err(`
    class Foo is
      public is
        function run() override end
      end
    end
  `, "has no parent"));

  it("親に同名メソッドがないのにoverride", () => err(`
    class Base is
      public is
        function greet() end
      end
    end
    class Child is Base
      public is
        function run() override end
      end
    end
  `, "no such method exists in parent"));

  it("overrideなしで親メソッドを再定義", () => err(`
    class Base is
      public is
        function greet() end
      end
    end
    class Child is Base
      public is
        function greet() end
      end
    end
  `, "missing 'override' keyword"));

  it("overrideでシグネチャ不一致（引数の数が違う）", () => err(`
    class Base is
      public is
        function greet(name: string) end
      end
    end
    class Child is Base
      public is
        function greet() override end
      end
    end
  `, "param(s) but parent has"));
});

describe("Checker: finalルール", () => {
  it("finalメソッドをoverride", () => err(`
    class Base is
      public is
        function build() final end
      end
    end
    class Child is Base
      public is
        function build() override end
      end
    end
  `, "cannot override final"));
});

describe("Checker: operatorルール", () => {
  it("operator + static はエラー", () => err(`
    class Foo is
      public is
        static function operator==(other: Foo)
          return false
        end
      end
    end
  `, "cannot be static"));
});

describe("Checker: 継承エラー", () => {
  it("存在しない親クラス", () => err(`
    class Foo is Bar
    end
  `, "unknown parent class"));

  it("クラスの重複定義", () => err(`
    class Foo is end
    class Foo is end
  `, "already defined"));

  it("循環継承", () => err(`
    class A is B end
    class B is A end
  `, "circular inheritance"));
});

describe("Checker: privateアクセス制御", () => {
  it("クラス外からprivateフィールドへの書き込みはエラー", () => err(`
    class Foo is
      private is
        Year = 0
      end
      public is
        static function new() end
      end
    end
    local f = Foo.new()
    f.Year = 2026
  `, "cannot access private field 'Year'"));

  it("クラス外からprivateメソッドの呼び出しはエラー", () => err(`
    class Foo is
      private is
        function explode() end
      end
      public is
        static function new() end
      end
    end
    local f = Foo.new()
    f.explode()
  `, "cannot access private method 'explode'"));

  it("クラス内からのprivateアクセスはOK", () => ok(`
    class Foo is
      private is
        Year = 0
        function explode() end
      end
      public is
        static function new() end
        function trigger()
          self.Year = 99
          self.explode()
        end
      end
    end
  `));

  it("別クラスからprivateメンバへのアクセスはエラー", () => err(`
    class Foo is
      private is
        Secret = 42
      end
      public is
        static function new() end
      end
    end
    class Bar is
      public is
        static function new() end
        function steal(f: Foo)
          return f.Secret
        end
      end
    end
  `, "cannot access private field 'Secret'"));

  it("LPL.mdのサンプル: L.Year = 2026はエラー", () => err(`
    class Lua is
      private is
        Year = 0
      end
      public is
        static function new(year: number) end
      end
    end
    local L = Lua.new(1993)
    L.Year = 2026
  `, "cannot access private field 'Year'"));

  it("LPL.mdのサンプル: L.explode()はエラー", () => err(`
    class Lua is
      private is
        function explode() end
      end
      public is
        static function new() end
      end
    end
    local L = Lua.new(1993)
    L.explode()
  `, "cannot access private method 'explode'"));
});

describe("Checker: abstractインスタンス化禁止", () => {
  it("abstractクラスをnewでインスタンス化するとエラー", () => err(`
    class Foo is abstract
      public is
        function explain() abstract
      end
    end
    local f = Foo.new()
  `, "cannot instantiate abstract class"));

  it("concreteクラスのインスタンス化はOK", () => ok(`
    class Foo is abstract
      public is
        function explain() abstract
      end
    end
    class Bar is Foo
      public is
        static function new() end
        function explain() override end
      end
    end
    local b = Bar.new()
  `));

  it("abstractクラスのnew呼び出しがメソッド内でもエラー", () => err(`
    class Base is abstract
      public is
        function explain() abstract
      end
    end
    class Maker is
      public is
        static function new() end
        function create()
          local x = Base.new()
        end
      end
    end
  `, "cannot instantiate abstract class"));
});

describe("Checker: override戻り値型チェック", () => {
  it("戻り値型が一致する場合はOK", () => ok(`
    class Base is
      public is
        function getValue(): number end
      end
    end
    class Child is Base
      public is
        function getValue(): number override end
      end
    end
  `));

  it("戻り値型の不一致はエラー", () => err(`
    class Base is
      public is
        function getValue(): number end
      end
    end
    class Child is Base
      public is
        function getValue(): string override end
      end
    end
  `, "has return type 'string' but parent has 'number'"));

  it("optional型の不一致はエラー", () => err(`
    class Base is
      public is
        function find(): string end
      end
    end
    class Child is Base
      public is
        function find(): string? override end
      end
    end
  `, "has return type 'string?' but parent has 'string'"));

  it("どちらか一方だけ型アノテーションがある場合はチェックしない", () => ok(`
    class Base is
      public is
        function run() end
      end
    end
    class Child is Base
      public is
        function run(): number override end
      end
    end
  `));
});

describe("Checker: superルール", () => {
  it("super.method()が親に存在する場合はOK", () => ok(`
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
  `));

  it("LPL.mdのC言語例: super.explain()が多段継承でOK", () => ok(`
    class Language is abstract
      public is
        function explain() abstract
      end
    end
    class Binary is Language
      public is
        function explain() override end
      end
    end
    class ASM is Binary
      public is
        function explain() override end
        function assembly() final end
      end
    end
    class C is ASM
      public is
        function explain() override
          super.explain()
        end
      end
    end
  `));

  it("親なしクラスでsuper使用はエラー", () => err(`
    class Foo is
      public is
        function run()
          super.run()
        end
      end
    end
  `, "has no parent"));

  it("親に存在しないメソッドのsuper呼び出しはエラー", () => err(`
    class Base is
      public is
        function greet() end
      end
    end
    class Child is Base
      public is
        function greet() override
          super.explain()
        end
      end
    end
  `, "does not exist in parent"));
});
