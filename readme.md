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

* 子クラスは自動で親のnewを継承する。

* operator関数はインスタンスメソッドとしてのみ定義可能である。
  staticとして宣言することはできない。(前述の通り)

* 演算子オーバーロード関数で想定していた型と異なる引数が渡された場合、
  false, nil, 0のいずれかを返すのが望ましい。

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

`!include`は`local`または`const`の単一行宣言でのみ使用できる。パスはinclude元からの相対`.luar`または`.lua`パスであり、循環include、ファイル欠落、末尾の`return <identifier>`不在はコンパイルエラーになる。

`.lua`はLua 5.4 parserで構文を検証してから、Lua 5.4 targetでは元の本文を保持してinline展開する。Lua 5.4の`<close>`や整数・ビット演算など、Luauで意味を保持できない機能をLuauへ出力しようとした場合は、近似変換せず互換性エラーにする。対象runtimeの標準ライブラリ差まではLuarが補完しない。

### 定義ファイル
`import type qaz`と書いた`.luar`ファイルと同じディレクトリに、`qaz.luard`を配置する。

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
`declare global name: Type`だけである。関数値は `declare run: (Arg1, Arg2) -> Return` と宣言し、引数・戻り値がない関数は `declare run: () -> ()` と書く。`.luar`本体に`declare`を書くことはできない。

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
- `.luard`は`import type`元と同じディレクトリだけから探索する。
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
