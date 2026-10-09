# 型システム

型注釈、`type` / `template`、ユニオン型、キャスト、型の推論と標準ライブラリ。実装は[architecture.md](../architecture.md)の「チェッカー」を参照。

## プリミティブ型と静的検査

ローカル変数、`const`、関数引数にはプリミティブ型を注釈できる。VS Codeではコロンの直後で`number`、`string`、`boolean`、`nil`、`table`、`function`、`any`を補完し、型名を専用の色で表示する。

```lua
local score: number = 0
local title: string = "Star Catcher"
local active: boolean = true
```

コンパイラは注釈とリテラルから確定できる型を追跡する。明らかに不正な初期化・代入、または演算子オーバーロードの引数型はコンパイルエラーになる。

```lua
local a: number = 1
local b: string = "A"
local c = a + b -- error: '+' はnumber同士だけに使用できる
```

`love.timer.getTime()`のような外部runtime由来の値は静的に型を断定できないため、推論だけで拒否しない。必要なら戻り値を`.luard`の宣言やローカル注釈で表す。

### 型宣言とジェネリクス(`type` / `template`)

`type`で型に名前を付けられる。`template <T>`は**直後の宣言1つだけ**に効く型引数で、`type`・`export type`・`declare function`・`function`定義(`local function`/`const function`も可)・`class`に付けられる。2つ目の宣言でも`T`を使うなら、その宣言にも`template <T>`が必要である。

```lua
template <T>
export type MyTable = { id: number, ref: T }

local t: MyTable<number> = { id = 1, ref = 2 }
local bad: MyTable<number> = { id = 1, ref = "x" } -- error: field 'ref' expects number, got string

template <T>
function first(a: T, b: T): T
    return a
end
first(1, "x") -- error: type parameter 'T' was inferred as number but argument 2 is string
```

型式は`Name<A, B>`、`mod.Name`、テーブル型`{ id: number, ref: T }`、関数型`(A, B) -> R`、`T?`を書ける。`type`と`export`と`template`は文脈依存のキーワードで、`type(x)`のような既存の識別子の使い方は変わらない。

`.luard`では`declare function`(`declare global function`も可)と`export type`を書ける。`export type`した型は`import type`した後に`mod.MyTable<number>`で参照する。`export`のない`type`は、その`.luard`の中だけで使える。`declare function`は`.luar`では使えない。

```lua
-- m.luard
template <T>
export type MyTable = { id: number, ref: T }

template <T>
declare function add(a: T, b: T): number

-- main.luar
import type m
local t: m.MyTable<string> = { id = 1, ref = "x" }
m.add(1, 2)
m.add(1, "a") -- error: type parameter 'T' was inferred as number but argument 2 is string
```

呼び出しの型検査は、`declare function`と`template`つきの関数だけが対象である。`template`のない通常の`function f(a: number)`の呼び出しは従来どおり検査しない。型の宣言内で未定義の型名(`template`を書き忘れた`T`など)はエラーになるが、通常の`local x: Foo`の未知の型名は従来どおりエラーにしない。

Luauへのコンパイルでは、型注釈をLuauの型構文のまま出力する。LuauのAnalyzeが引数や戻り値の型を推論できるようになり、`--!native`を使う場合は型に応じた最適化も効く(通常の実行速度は変わらない)。Lua 5.4へのコンパイルでは型を消去し、元の宣言を直前の1行コメントとして書き出す。

```lua
-- Luau
export type MyTable<T> = { id: number, ref: T }
local score: number, name = 0, "x"
function first<T>(a: T, b: T): T ... end

-- Lua 5.4
-- export type MyTable<T> = { id: number, ref: T }
-- local score: number, name
local score, name = 0, "x"
-- function first<T>(a: T, b: T): T
function first(a, b) ... end
```

- 出力コードの中に宣言が無い型名(クラス、`import type`したモジュールの型、`declare class`の型)は`any`に置き換える。`Dog?`は`any?`、`{ Dog }`は`{ any }`になる。
- クラスのメソッドも引数と戻り値の型を出力する(`self`は無注釈)。クラス自体の型引数(`template <T> class Box`)は`any`になる。
- `local f: (number) -> number = function(x) ... end`のように関数式を代入するときは、`local function f`へ変換するため`f`の型注釈は出力されない(引数と戻り値の注釈は残る)。

### ユニオン型(`A | B`)

`A | B`で「AまたはB」の型を表す。`type`で名前を付けたり、引数・戻り値・`::`に使える。`T | nil`は`T?`と同じ。

```lua
type Arithmetic = Vector3 | number

declare class Vector3 is
    public is
        function operator*(a: Vector3, b: Vector3 | number): Vector3
        function operator/(a: Vector3, b: Arithmetic): Vector3
    end
end
```

ユニオンを代入するには、すべてのメンバーが受け側に入る必要がある(`number | string`は`number`に入らないが、`number | string | boolean`には入る)。`::`はどれか1つのメンバーと関連していれば許可する。ユニオンの値への演算子は、どのメンバーかを追跡しないため検査しない。関数型の戻り値は`|`を含んで読む(`(A) -> B | C`は戻り値が`B | C`)。

`template <T>`はC++と同じく、直後の宣言1つだけに効く。演算子メソッドにも付けられるが、複数の演算子で共有したいときは、それぞれに書くか、ユニオン型で表す(上の例)。`.luard`の`declare class`は、`declare global class`とは書かない(クラス名はもともと修飾なしで使える)。

### 型キャスト(`::`)

`式 :: 型`で、式の型を確定させる。

```lua
local a: string = "2"
local b = tonumber(a) :: number   -- number? を number にする

local i = Instance.new("Part")    -- Instance?
local p = i :: Part               -- 派生クラスへのダウンキャスト
local s = 1 :: string             -- error: cannot cast number to string
```

キャストできるのは**関連する型**だけである。元と先の一方が他方へ代入できる、継承関係にある(派生へのダウンキャストを含む)、`T?`から`T`への絞り込み、型が決まらない値のいずれか。`number`と`string`のような無関係な型や、継承関係のないクラス同士はエラーになる。テーブル(`setmetatable({}, Dog)`など)はクラスとして扱える。

- 二項演算子より強く、単項演算子より弱く結び付く。`a + b :: number`は`a + (b :: number)`、`(a + b) :: number`は括弧で束ねた式へのキャストである。
- `::name::`はラベルなので、`::`の後に識別子と`::`が続く並びはキャストにならない。キャストの連鎖は括弧で書く(`(x :: any) :: number`)。
- 型の側の`<`はジェネリクスとして読まれる。`a :: number < 3`ではなく`(a :: number) < 3`と書く。
- Luauへのコンパイルでは、キャストを括弧で束ねた`(tonumber(a) :: number)`として出力する。Lua 5.4へのコンパイルでは、キャストを消去し、その文の直前に`-- cast: tonumber(a) :: number`というコメントを付ける。

### 型の推論と標準ライブラリ

注釈がなくても、コンパイラが確定できる型は確定させる。リテラル・演算・`#`・`..`・比較・`x or default`・`new`・テーブルの形に加え、次を推論する。

- 標準ライブラリの戻り値。`math.floor(x)`は`number`、`tonumber(s)`は`number?`、`("x"):upper()`は`string`。
- 注釈のないユーザー関数の戻り値。すべての`return`を集め、`return 1`なら`number`、`return 1`と`return nil`が混在すれば`number?`、型が食い違えば不明な型になる。注釈の戻り値があればそれを優先する。ユーザー関数の引数は、`template`のない通常の関数では従来どおり検査しない。

Lua 5.4とLuauの標準ライブラリ(`print`・`tostring`・`math`・`string`・`table`・`os`・`utf8`・`coroutine`、Lua 5.4の`io`、Luauの`bit32`・`task`・`typeof`など)は型つきで登録してあり、`--target`に応じて内容が変わる。標準関数は**引数の個数と型も検査する**。

```lua
math.floor("x")      -- error: argument 1 of 'floor' expects number, got string
string.rep("x")      -- error: function 'rep' expects at least 2 argument(s), got 1
math.max(1, 2, 3)    -- 可変長
```

`local print = ...`のように自分で宣言した名前は標準ライブラリを隠し、`.luard`で宣言した名前は標準ライブラリより優先される。`love.timer.getTime()`のような型が決まらない値は、どの標準関数にも渡せる。

VS Codeでは標準ライブラリの名前を`defaultLibrary`修飾子つきで色付けする(`print`・`tonumber`は関数、`math`・`string`は名前空間、`math.floor`は関数、`math.pi`は読み取り専用の変数)。拡張機能側の更新(VSIXの再生成と再インストール)が必要である。
