# Luar Programming Language

## コンパイラの更新（Windows）
開発中にコンパイラを更新するときは、リポジトリ直下から次を実行する。

```powershell
.\tools\Update-Luar.ps1
```

このスクリプトはRelease版をビルドし、`%USERPROFILE%\.cargo\bin\luar.exe`（`CARGO_HOME`を設定している場合は`%CARGO_HOME%\bin\luar.exe`）へ置き換える。更新結果は次で確認できる。

```powershell
luar --version
Get-Command luar -All
```

`luar-rs\target\release`はビルド成果物であり、PATHへ登録しない。PATHにはCargoの`bin`ディレクトリだけを登録して、常に更新スクリプトが置き換える実行ファイルを使用する。

VS Code拡張はPATH上の`luar`へ未保存の本文を渡し、Rustコンパイラと同じ診断を表示する。別の実行ファイルを使う場合は`luar.compiler.path`、互換性検査の対象は`luar.target`（`luau`または`lua54`）で設定する。コンパイラが見つからない場合もハイライトと補完は利用できるが、意味診断は無効になる。

### 補完
`.`(`:`)を打つと、VS Codeは`luar complete`でコンパイラのチェッカーへ問い合わせ、レシーバの型に応じたメンバーを種別(field/method/function)と型付きで提案する。識別子の補完は、その位置から見えるlocal・const・関数・クラス・モジュールを、種別(`local x: number`、`const NAME: string`など)と型付きで返す。外部から見えるのはpublicメンバーだけで、クラス自身(`Dog.`)にはstaticメソッドと`new`、インスタンス(`dog.`)にはフィールドとインスタンスメソッドが出る。

```powershell
luar complete --stdin --source-path main.luar --offset 123 < main.luar
```

`--offset`は文書先頭からのUTF-16コード単位のオフセットで、結果は`{"items":[{"label","kind","type","detail"}]}`のJSONである。入力途中で閉じ括弧が足りない場合も、補った版で解析する。コンパイラを呼べない・結果が空のときは、従来のこのファイル内の索引へフォールバックする。クラスのメソッド本体の中(`self.`など)の補完は未対応である。

### 色分け(semantic tokens)と定義ジャンプ
キーワードや文字列などはTextMate文法で、変数・const・引数・関数・メソッド・フィールド・クラス・モジュール(`import type`と`!include`の束縛名)・型名は、コンパイラの解析結果で色分けする(`luar tokens`)。constと宣言位置には`readonly`/`declaration`、staticメソッドには`static`の修飾が付く。`.luard`のクラス名もクラスとして色付けされる。拡張機能は`[luar]`/`[luard]`でsemantic highlightingを既定で有効にする。

Ctrlクリック(F12)の定義ジャンプは`luar definition`で行う。対象は次のとおり。

- 同じファイルの`local`/`const`/関数/引数/クラス/メソッド/フィールドの宣言(スコープとシャドーイングを考慮)。
- `!include`したファイルの宣言。`clsdef.dog.name`の`dog`/`name`は、レシーバの型からインクルード先の定義へ飛ぶ。
- `import type`した`.luard`の`declare`/`declare class`とそのメンバー。`import type`の名前は`.luard`自体を開く。
- `!include("./x.luar")`のパス文字列と、その束縛名は`x.luar`を開く。

```powershell
luar tokens --stdin --source-path main.luar < main.luar
luar definition --stdin --source-path main.luar --offset 123 < main.luar
```

型が決まらないメンバー参照は、同名のメンバー宣言を全て候補として返す。色分けと定義ジャンプは字句解析ができれば動くため、入力途中の構文エラーがあるファイルでも使える。

拡張機能をVSIXから利用している場合、ソース変更後はビルドだけでなくVSIXの再生成と再インストールが必要になる。次のスクリプトがVSIXの生成からインストールまでを行う。`-WithCompiler`を付けると`luar.exe`の更新も同時に行う。実行後はVS Codeを再読み込みする。

```powershell
.\tools\Update-LuarExtension.ps1
.\tools\Update-LuarExtension.ps1 -WithCompiler
```

```powershell
cd .\luar-vscode
npm run package
code --install-extension .\luar-language-0.1.0.vsix --force
```

## lintと外部runtime

構文エラーと未定義globalの診断は、原因となるtoken全体を赤線または黄線で示す。未定義globalはLuaのruntime依存値を扱えるようwarningであり、`luar check`の終了コードを失敗にしない。Lua/Luau標準globalはあらかじめ認識する。

外部runtimeは設定項目ではなく、通常の`.luard`で宣言して使い回す。

```lua
-- love.luard
declare global love: Love

-- main.luar
import type love
love.graphics.print("hello")
```

`import type love`は宣言だけを読み、`require`などの実行時コードを生成しない。未宣言の`love`や綴り誤りの`lovve`はwarningになる。

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

型は生成コードでは消去されるが、元の宣言を直前の1行コメントとして書き出す(Luau/Lua 5.4共通)。コンパイラの不具合調査用である。

```lua
-- export type MyTable<T> = { id: number, ref: T }
-- local score: number, name
local score, name = 0, "x"
-- function first<T>(a: T, b: T): T
function first(a, b) ... end
```

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
- 生成コードではキャストを消去し、その文の直前に`-- cast: tonumber(a) :: number`というコメントを付ける。

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

## 概要
Luau言語から派生し、ついにオブジェクト指向・テーブルのディープコピーを実現。

## コンパイルターゲット

LuarはLuauとLua 5.4へ出力できる。既定ターゲットは後方互換のため`luau`であり、出力ファイルの拡張子からターゲットを推測しない。

```powershell
luar compile --target luau main.luar main.luau
luar compile --target lua54 main.luar main.lua
luar check --target lua54 main.luar
luar dump-ir main.luar
```

コンパイラは制御構造をLuar独自の制御フローIR（CFIR）へ変換してから、対象言語のソースへ再構成する。`dump-ir`はこの中間表現を調査する開発用コマンドであり、その表示形式は安定した公開APIではない。

`luar-rs`が唯一の正式なコンパイラ実装である。`luar/`以下のTypeScript frontendはVS Codeの寛容な編集中インデックスと互換テストのために残されているが、CLIおよびcodegenとしては非推奨である。

## ブロックif式

`if`は式としても使用できる。各branchには0個以上の文を書け、最後の値式がそのbranchの値になる。値を必ず生成するため、`else`は必須である。

```lua
local x = if cond then
    print("working")
    123
elseif other then
    prepare()
    456
else
    789
end
```

短い形式や、関数引数・二項演算子・`return`の中でも使用できる。

```lua
local short = if enabled then 1 else 0 end
foo(if enabled then 20 else 30 end)
return 100 + if enabled then calculate() 1 else 2 end
```

branch末尾の関数呼び出しは副作用を持つ文として扱われるため、値を返す場合も最後に値式を置く。例えば`print("debug")`の後に`1`を置く。branchの値型は既存の型検査で比較され、`number`と`string`など互換性のない組み合わせはエラーになる。Luauでは文を含まない単純なif式をnative if expressionへ出力し、それ以外とLua 5.4ではtemporaryとif文へloweringする。`a`が`false`や`nil`でも意味を壊す`a and b or c`への変換は行わない。

## 条件付きbinding（`:=`）

`if`、`elseif`、`while`の条件では、bare identifierに`:=`を使って値を一度だけ評価し、新しいlocalへ束縛できます。

```lua
if child := parent:FindChild("ABCD") then
    print(child.Name)
elseif fallback := getFallback() then
    print(fallback)
end

while line := file:ReadLine() do
    print(line)
end
```

bindingは条件のtruthy branchと`while`本体でだけ有効です。`else`、後続の`elseif`条件、`if`の後へは漏れず、外側に同名のlocalがあれば内側でシャドーイングします。`nil`と`false`は偽、`0`と空文字列は真というLua/Luauのtruthinessに従います。`:=`の右辺は各条件評価につき一度だけ実行されます。

`:=`は通常の式では使えず、左辺は単一の識別子に限られます。`object.field := value`、`print(x := value)`などはコンパイルエラーです。optional型の値はtruthy branchで既存の型refinementが適用されます。backendではLuau/Lua 5.4とも、必要な`do`・temporary・`break`へloweringし、`:=`自体は出力しません。

## gotoとcontinue

LuarではLua互換のlabelと`goto`を使用できる。

```lua
function main()
    for i = 1, 10 do
        for j = 1, 10 do
            if i == 1 and j == 3 then
                goto exit
            end
            print(i, j)
        end
    end
    print("end for loop")
    ::exit::
    print("exit")
end
```

- labelは同じ関数またはchunk内の可視なblockに存在しなければならない。
- 同じscopeの重複label、未定義label、ローカル変数のscope内へ飛び込む`goto`はコンパイルエラーになる。
- `break`と`continue`はloop内でのみ使用できる。
- Lua 5.4出力ではnative `goto`を利用する。Luau出力では可能な限り構造化し、一般的な`goto`が残る領域だけをdispatcherへ変換する。

現時点のLuau dispatcherは、関数またはchunk直下に置くlabelを対象にしている。上のようにloop内から直下のlabelへ抜ける形式は利用できる。一方、入れ子block内のlabelと、直下のlocal宣言をまたいで状態を保持するlabelは、意味を近似しないため診断する。Lua 5.4ではnative `goto`を生成するが、Luar frontend側の入れ子labelの解析は次のCFIR拡張で扱う。

## フォーマット文字列

バッククォート文字列は、埋め込み式をそれぞれ一度だけソース順に評価し、`tostring`相当で文字列化して結合する。

```lua
local name = "Luar"
local version = 1
print(`Hello {name} {version}`)
```

Luauではnativeなバッククォート文字列として、Lua 5.4では意味が等価な生成コードとして出力される。literalなbraceとバッククォートは`\{`、`\}`、``\` ``でescapeする。

## const
Luau 0.731と同じconst宣言を使用できる。

```luau
const VERSION: string = "1.0"
const WIDTH, HEIGHT = 1280, 720

const function area(): number
    return WIDTH * HEIGHT
end
```

- const宣言には初期値が必要であり、宣言後の再代入は禁止される。
- constが参照するテーブルなどの値自体は不変化されない。
- 内側のスコープで同じ名前を宣言するシャドーイングは可能。
- `const`は文脈依存キーワードであり、フィールド名などでは通常の識別子として使用できる。

## OOP
```luau
class Lua is
    -- public, privateブロックの外にあるメソッド、変数はデフォルトでprivate
    private is
        Year = 0
        function explode()
            print("Boom!!")
        end
    end

    public is
        static function new(year: number) -- コンストラクタ(予約語)
            self.Year = year
        end

        static function panic() -- static関数(インスタンスからは呼べない)
            print("I'm not a crab!!!")
        end

        function greet()
            print(`Hello from {self.Year}!`)
        end

        function operator==(AnotherRock: Lua)
            return false -- Luaは純血です
        end

        function free() -- デストラクタ(予約語)
            print("Goodbye world")
            -- self = nil
        end
    end
end

local L = Lua.new(1993)
local L2 = Lua.new(2006)
L.greet()
print(L == L2) -- false
Lua.panic()
-- L.explode() -- コンパイルエラー(privateへのアクセス)
-- L.Year = 2026 -- コンパイルエラー(privateへのアクセス)
L.free() --[[
このとき、参照の破棄、イベントの切断、ループ停止（並行処理など）などが行われる。
メモリを解放はしない。
]]
```

### コンパイル後(例)
```luau
-- クラステーブル
local Lua = {}
Lua.__index = Lua

-- メタメソッド（operator==）
Lua.__eq = function(self, AnotherRock)
    return false
end

-- ========================
-- private領域（外から触れないようローカル化）
-- ========================

local function explode(self)
    print("Boom!!")
end

-- ========================
-- public static関数
-- ========================

function Lua.new(year)
    local self = setmetatable({}, Lua)
    self.Year = 0 -- 初期値

    -- コンストラクタ処理
    self.Year = year

    return self
end

function Lua.panic()
    print("I'm not a crab!!!")
end

-- ========================
-- public インスタンスメソッド
-- ========================

function Lua.greet(self)
    print(`Hello from {self.Year}!`)
end

function Lua.free(self)
    print("Goodbye world")
    -- クリーンアップ処理（ユーザー定義）
end

-- ========================
-- 使用コード
-- ========================

local L = Lua.new(1993)
local L2 = Lua.new(2006)

L:greet()

print(L == L2) -- false

Lua.panic()

-- L:explode() -- アクセス不可（ローカル関数なので）

-- L.Year = 2026 -- ⚠ Luau的には防げない（仕様的にはエラーだが実行時は通る）

L:free()
```

### 継承
```Luau
class Language is abstract
    year = 1901
    public is
        function explain() abstract
    end
end

class Binary is Language
    year = 1930
    public is
        function explain() override
            print("made by 0 and 1")
        end
    end
end

class ASM is Binary
    year = 1940
    public is
        -- 親クラスと同じ名前のメソッドをoverrideなしで書くことは禁止されている
        -- function explain()
        --     print("made with ASCII")
        -- end

        function explain() override
            print("made with ASCII")
        end

        function assembly() final
            print("B0 0A >>> MOV AL, 10")
        end
    end
end

class C is ASM
    year = 1972
    public is
        function explain() override
            super.explain() -- ASM.explain()
            print("made with English")
        end

        -- finalによって禁止されている
        -- function assembly() final
        --     print("B0 0A >>> char A = 10;")
        -- end
    end
end
```
#### 継承の仕様
- overrideキーワードを使用する場合、
  親クラスに同名のメソッドが存在しなければならない。

- 親クラスに同名のメソッドが存在する場合、
  **overrideキーワードなしでの再定義は禁止される。**

- overrideはメソッドにのみ適用される。
  フィールドに対して使用することはできない。

- フィールドは親クラスのものを自由に上書きできる。
  overrideキーワードは不要である。


- overrideするメソッドは、
  親クラスのメソッドとシグネチャ（名前・引数・戻り値）が
  **完全一致していなければならない。**

- abstractクラスはインスタンス化できない。

- abstractメソッドを持つクラスは、abstractでなければならない。

- abstractメソッドを実装する場合もoverrideキーワードを必須とする。

- finalキーワードはメソッドにのみ適用可能であり、
  子クラスでオーバーライドすることを禁止する。

- superは直前の親クラスに定義されたメソッドを呼び出す。
  多段継承の場合、親クラス側でsuperが呼ばれない限り、それ以上は遡らない。

### 備考

* コロンとドットは使い分けません。なぜか？コンパイラが自動でselfを使うように変換します。
  その方が理不尽でわかりづらいエラー（引数ずれ）とかが起きませんよね？

* static以外のメソッドは常にselfを受け取る。
  しかし、**演算子オーバーロード関数はselfが必要なため、staticとして宣言はできない。**

* newは特別なstatic関数であり、以下のように変換される：

  1. 新しいテーブルを生成する
  2. クラスのメタテーブルを設定する
  3. そのインスタンスをselfとして関数を実行する
  4. selfを返す

* クラス直下およびフィールド定義は、インスタンス生成時に各インスタンスへコピーされる。

* `new`を宣言しないクラスには、コンパイラが引数なしのpublicなデフォルトコンストラクタ`new()`を生成する。フィールドの初期値だけが代入される。
  `private`な`new`だけを宣言したクラスにはデフォルトコンストラクタを足さない。abstractクラスにも生成されるが、直接のインスタンス化は従来どおりエラーになる。

```luau
class Dog is
    public is
        name: string = "Pochi"
    end
end

print(Dog.new().name) -- Pochi
```

* 子クラスは自動で親のnewを継承する。親（祖先）が`new`を宣言していなければ、親のデフォルトコンストラクタを呼んだうえで子のフィールド初期値を代入する。

* operator関数はインスタンスメソッドとしてのみ定義可能である。
  staticとして宣言することはできない。(前述の通り)

* 演算子オーバーロード関数で想定していた型と異なる引数が渡された場合、
  false, nil, 0のいずれかを返すのが望ましい。

* `friend class X`をクラス直下に書くと、クラスXにこのクラスのprivateメソッド・フィールド(private `new`を含む)へのアクセスを許可する。
  許可は片方向で、継承されない(Xの子クラスは許可されない)。`friend`は`friend class`と続くときだけキーワードで、それ以外では通常の識別子として使える。存在しないクラスを指定するとエラーになる。
  friendが指定されたクラスのprivateメソッドは、他のクラスから呼べるようクラステーブルにも載せて出力する(`Vault.audit = audit`)。

```luau
class Vault is
    friend class Teller
    private is
        balance = 10
    end
end

class Teller is
    public is
        function peek(): number
            return Vault.new().balance -- friendなのでOK
        end
    end
end
```

* public / privateによるアクセス制御はコンパイル時にのみ適用される。
  実行時には追加のオーバーヘッドは発生しない（ゼロオーバーヘッド抽象化）。

* privateメンバは実行時には通常のフィールドとしてインスタンスに格納されるが、
  コンパイラによってクラス外からのアクセスは禁止される。

* これらの制約はLuarコンパイラによって保証されるものであり、
  **生成されたLuaコードや手動で記述されたLuaコードには適用されない。**

## テーブルコピー
```luau
local t = {
    1,
    2,
    3,
    {
        "a",
        "b",
        "c"
    }
}

local t2 = table.deepcopy(t)
print(t2) --[[
{
    1,
    2,
    3,
    {
        "a",
        "b",
        "c"
    }
}
]]
```

### 備考
- ポインタはコピーせずに参照のまま(外部オブジェクトとか)
    **metatableはコピーされない**
    userdata / function / thread は参照
- 循環は同じ参照にし、一度追加したものはパスする。
- この関数はLuar標準に組み込まれる。(Luauのtableライブラリをmodする)
- もしくは、展開して貼り付け(実用性というか)

## モジュール(import type)
`import type`はコンパイル時の名前解決だけを行う型専用宣言であり、実行時にmoduleを読み込む機能ではない。旧 `import module` 構文は使用できない。

### 実体の束縛
`import type mod`は、実行時の変数`mod`を作らない。これは`.luard`の宣言を参照し、未修飾のメンバー名を`mod.member`へ解決するためだけの名前である。

実行時のモジュールテーブルは、対象ランタイムに合わせて同名の`local`または`const`で束縛する。これは意図的なシャドーイングであり、コンパイルエラーにはならない。

```luau
import type mod
const mod = require("@./mod.luar")

mod.run()
print(hogehoge) -- mod.hogehogeへ変換
```

`require`、C/C++バインド、グローバルテーブルなど、実体をどのように提供するかはLuarではなく実行環境が決める。`import type`自体は実行時コードを生成しない。

### ソースのインライン展開
実行環境の`require`が`.luar`を読めない場合は、宣言形式の`!include`マクロを使用できる。

```luau
import type mod
local mod = !include("@./mod.luar")

mod.run()
print(hogehoge)
```

`!include`は、相対パスの`.luar`をコンパイル前にインライン展開する。取り込むファイルは最後にテーブル変数を返す必要がある。

`!include`だけでモジュールの型と全メンバーはソースから分かるため、`import type`も`.luard`も不要である。`import type`が必要になるのは、ソースが見えない実行時提供のモジュール(loveなど)と、未修飾名の自動修飾(`print(hogehoge)`→`mod.hogehoge`)を使うときだけである。同名の`!include`がある`import type`は`.luard`を要求せず、includeしたソースのトップレベルのテーブルから宣言を作る。`.luard`も`!include`もない`import type`は従来どおりエラーになる。

```luau
local clsdef = !include("./clsdef.luar")
print(clsdef.dog.name) -- clsdef.dog: Dog, name: string と推論される
```

#### includeした名前の衝突
展開したソースのトップレベルの名前(`local`/`const`/`function`/`class`)が、include元が束縛している名前(別の`!include`で取り込んだものを含む)と同じ場合、include側の名前だけを`<元の名前>__<束縛名>`へ自動で改名する(例: `module`→`module__clsdef`)。使われているだけで束縛されていない名前は、includeしたモジュールが提供する名前とみなして改名しない。メンバー名、テーブルのキー、ラベル、文字列、クラスのメンバー宣言名は改名されない。改名が必要な名前がテンプレート文字列の`{}`内で使われている場合は、安全に改名できないためコンパイルエラーになる。

```luau
-- mod.luar
local mod = {}
mod.hogehoge = "gepyaaa"

function mod.run()
    print("this is mod, not admin!")
end

return mod
```

上の例は次のLuarソースへ展開される。末尾の`return mod`は除去され、代入先と戻り値名が異なる場合だけ別名を束縛する。

```luau
local mod = {}
mod.hogehoge = "gepyaaa"

function mod.run()
    print("this is mod, not admin!")
end

mod.run()
print(mod.hogehoge)
```

includeされるファイルは、末尾の`return <名前>`で、モジュールとして渡すものを決める。テーブルだけでなく、`class`の名前も返せる。`static`メソッドだけのクラスは、名前空間のように使える。

```lua
-- table2.luar
class table2 is
    public is
        static function add(items: {number}, amount: number)
            -- ...
        end
    end
end

return table2
```

末尾の`return`が無いときのエラーには、ファイルで最後に宣言されたクラス(なければ`local`/`const`/`function`)の名前を使った`return`の例が付く。

`!include`は`local`または`const`の単一行宣言でのみ使用できる。パスはinclude元からの相対`.luar`または`.lua`パスで、`..`やサブディレクトリを含められる(絶対パスは使えない)。includeされたファイルの中の`!include`は、そのファイルからの相対パスである。循環include、ファイル欠落、末尾の`return <identifier>`不在はコンパイルエラーになる。

`.lua`はLua 5.4 parserで構文を検証してから、Lua 5.4 targetでは元の本文を保持してinline展開する。Lua 5.4の`<close>`や整数・ビット演算など、Luauで意味を保持できない機能をLuauへ出力しようとした場合は、近似変換せず互換性エラーにする。対象runtimeの標準ライブラリ差まではLuarが補完しない。

### 定義ファイル
`import type qaz`と書いた`.luar`ファイルと同じディレクトリに、`qaz.luard`を配置する。別のディレクトリの定義ファイルを使うときは、`from`でパスを書く。

```lua
import type love from "../defs/love.luard"
print(love.timer.getTime())
```

パスは`import type`を書いたファイルからの相対パスで、`..`とサブディレクトリをいくつでも含められる。拡張子は`.luard`だけで、絶対パスは使えない(`!include`と同じ規則)。`love`は修飾名で、ファイル名とは無関係に付けられる。エディタは、このパスの解決をコンパイラ(`luar imports`)に問い合わせる。

```luau
-- main.luar
import type qaz
local qaz = require("@./qaz.luar") -- 実体の読み込みは対象ランタイムに合わせて書く

print(wsx)       -- qaz.wsxへ変換
print(workspace) -- global宣言なので変換しない
```

```luau
-- qaz.luard
declare wsx: string
declare global workspace: workspace
```

`.luard`に記述できるものは、コメント、空行、`declare name: Type`、
`declare global name: Type`、`declare class`だけである。関数値は `declare run: (Arg1, Arg2) -> Return` と宣言し、引数・戻り値がない関数は `declare run: () -> ()` と書く。`.luar`本体に`declare`を書くことはできない。

#### declare class
ソースのないクラス(C/C++バインドなど)にも、補完と静的検査を効かせるために`declare class`でクラスの形を宣言できる。メンバーはすべてpublicで、本体を持たない。継承は`is Parent`だけ書ける。

```luau
declare class Dog
    name: string
    function bark(times: number): string
    static function create(): Dog
end
declare dog: Dog
```

`class Name is`と同じ構文(`is`あり)でも書ける。この形式ではpublic/privateブロック、メソッド本体、`friend`をそのまま使える。本体は構文だけ検査し、コードは生成しない。privateメンバーは外部から使えず、補完にも出ない。

```luau
-- Part.luard
declare class Part is
    friend class Instance
    public is
        Anchored = false
        CanCollide = true
    end
    private is
        static function new()
        end
    end
end

declare class Instance is
    public is
        static function new(name: string)
            return Part.new()
        end
    end
end
```

```luau
-- main.luar
import type Part

if part := Instance.new("Part") then
    part.Anchored = true
end
```

`declare name: Type`の`Type`にはクラス名を書ける。宣言したクラス名は修飾せずそのまま使え、実行時コードは生成されない。`import type ext`したあとで`local ext = require(...)`と実体を束縛しても、`.luard`が宣言した型は保たれる。

### 型推論の範囲
テーブルリテラル`{ dog = Dog.new() }`は、フィールド名と型が分かるテーブルとして扱う。`local m = {}`のあとの再代入や`m.x = 1`、`function m.run()`でも形が更新される。`obj.field`は、形が分かるテーブルとクラスのインスタンスに対して型を返し、メソッド呼び出しは戻り値の型注釈があるときだけ型が確定する。判断できないものは`any`相当(Unknown)のままで、外部runtime由来の値を推論だけで拒否することはない。フィールド型は注釈があればそれを、なければ`"Pochi"`のようなリテラルから推論する。

### 名前の衝突
```luau
-- main.luar
import type hoge
import type foo

print(bar)      -- コンパイルエラー: hoge.barとfoo.barのどちらか曖昧
print(hoge.bar) -- OK
```

```luau
-- hoge.luard
declare bar: string
```

```luau
-- foo.luard
declare bar: number
```

### 仕様
- `import type name`は、`import type`元と同じディレクトリの`name.luard`を読む。別の場所は`import type name from "相対パス.luard"`で指す。
- importしたmoduleの非global宣言に一致する、ソース内で束縛されていない未修飾名は
  `[module名].[フィールド名]`へ変換する。
- local、const、関数引数、ループ変数、関数名、クラス名はmodule宣言より優先される。
- `declare global`で宣言された名前と、どのmoduleにも宣言されていない名前は未修飾のままにする。
- 同じ名前が複数moduleに存在しても、その未修飾名が実際に使われなければエラーにしない。
  実際に使われた場合だけ曖昧な参照としてコンパイルエラーにする。
- `hoge.bar`のような修飾済み参照は曖昧性の影響を受けない。
- moduleは実行時にテーブルとして提供されている必要がある。実体の束縛については「実体の束縛」を参照する。
- 定義ファイルの欠落、不正な記述、同一ファイル内の重複宣言、同じmoduleの重複importは
  ファイル名と行番号を含むコンパイルエラーになる。

### 例

ソースコード例1(名前修飾の衝突なし):
```luau
import type qaz
local qaz = require("@./qaz.luar")

print(wsx)
print(workspace)
```

```luau
declare wsx: string
declare global workspace: workspace
```
---
ソースコード例2(名前修飾の衝突あり):
```luau
import type hoge
import type foo

print(bar) -- 曖昧!!
print(hoge.bar) -- OK
```

hoge:
```luau
declare bar: string
```

foo:
```luau
declare bar: number
```
---
### 仕様(原文)
- コンパイラは、名前修飾がなく、ソース内で定義されていない変数がある場合、
  モジュールを参照し、定義がある場合はそのままコンパイルを通す。

- 名前修飾が衝突している場合、コンパイルエラーとする

- 実際のモジュール読み込みはLuau処理系に依存する(
    例:
    - C/C++によってバインドされている場合
    - require関数を使用する(Roblox)
    - コピペしてめちゃくちゃにする
    )
   コンパイラは干渉しない。

- モジュールはテーブルで書かれているべきである。
  なぜなら、コンパイラが`[モジュール名].[フィールド名]`に置き換えるため。
  グローバルアクセスが可能で衝突がなければglobal宣言すればそのままになる。
  万が一グローバル変数が衝突する場合、Luauソースにこのような変換層を生成する:
  ```luau
  -- import type Love
  -- import type Roblox
  local LUAR__RAW_workspace = workspace --この環境で定義されている変数を保存する
  local Love = {
    workspace = LUAR__RAW_workspace
    numnum = 100
  }
  local Roblox = {
    workspace = LUAR__RAW_workspace
    numnum = 2006
  }

  print(Love.workspace)
  print(Roblox.numnum)
  ```

## 処理フロー
`Luar/Luaソース-[frontend]->HIR-[制御フローlowering]->CFIR-[Luau backend]->Luauソース->LuauVM`

または:

`Luar/Luaソース-[frontend]->HIR-[制御フローlowering]->CFIR-[Lua 5.4 backend]->Luaソース->Lua 5.4 VM`
### あとがき
mixinも追加予定(今回は実装しない)
mixinってなんですか？多分不要では
