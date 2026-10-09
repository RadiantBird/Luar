# 構文

ブロックif式、条件付きbinding、goto/continue、フォーマット文字列、const、テーブルコピー(構想)。
クラスは[classes.md](classes.md)、モジュールは[modules.md](modules.md)を参照。

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

## テーブルコピー
> 状態: **未実装(構想)**。`table.deepcopy`はコンパイラにも標準ライブラリの定義にも存在しない。以下は設計メモ。

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
