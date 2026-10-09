# クラス(OOP)

クラス、継承、アクセス制御、デストラクタ`free`と`using`による自動解放。
生成コードの形は[architecture.md](../architecture.md)の「コード生成」を参照。

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

function Lua.__free(self)
    print("Goodbye world")
    -- クリーンアップ処理（ユーザー定義）
end

function Lua.free(self)
    if rawget(self, "__freed") then return end -- 2回目以降は何もしない
    self.__freed = true
    Lua.__free(self)
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

### スコープを抜けるときの自動解放(`using`)
`using`で宣言した変数は、スコープを抜けるとき`free()`が自動で呼ばれる。イベントの切断のような「必ず後始末したい」処理に使う。`local`は今までどおり何もしない。

```luau
using conn = Signal.new() -- このブロックを抜けるとき conn.free() が呼ばれる
conn.connect(handler)
if failed then
    return -- returnでも、break/continueでも、error()でも呼ばれる
end
```

- `using 名前 [: 型] = 式`の形で、束縛は1つ、初期化式は必須。`const`と同じで再代入はできない。`using`は変数名にも使える(`using x =`の形のときだけ宣言になる)。
- 値は`free`を持つクラス(祖先の`free`でもよい)のインスタンスでなければコンパイルエラー。`Class?`のときは、nilなら何もしない。
- 複数あるときは宣言の逆順に解放する。`return`の値は、解放の前に評価する。
- `free()`は何度呼んでも1回しか実行しない。明示的に`free()`を呼んだ変数が、スコープを抜けるときにもう一度実行されることはない。
- 継承したクラスでは、子の`free`のあとに親の`free`が自動で呼ばれる(`super.free()`は書かなくてよい)。
- 制約: `using`のある関数では`goto`とラベルは使えない。`repeat ... until`の本体の直下、if式の中には書けない。

Luauへのコンパイルでは、`using`以降の文を`pcall`で包み、抜けたあとで`free()`を呼ぶ。`return`/`break`/`continue`は包んだ関数の戻り値で外へ中継するので、スコープごとにクロージャが1つ増える。Lua 5.4へのコンパイルでは`local conn <close> = ...`になり、各クラスに`__close`メタメソッドを出力する。

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
