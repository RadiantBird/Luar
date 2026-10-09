# モジュール(`import type` / `!include` / `.luard`)

型だけを読み込む`import type`、ソースを取り込む`!include`、宣言ファイル`.luard`。
`import type`は実行時にモジュールを読み込まない。実体はどう提供するかを実行環境が決める。

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
