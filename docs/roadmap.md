# ロードマップと既知の制限

実装していないこと、意図して見送ったこと、今後の候補。解消したら項目を消し、理由を[design-decisions.md](design-decisions.md)か[progress.md](progress.md)に残す。

## 既知の制限(仕様どおりで、まだできないこと)

| 項目 | 内容 | 補足 |
|---|---|---|
| クラスの型をLuauの型として出力 | クラス名・`import type`の型・`declare class`の型は、Luau出力では`any`になる | 型を厳密に出すには、クラスごとに`type`宣言(`typeof(setmetatable(...))`など)を生成する必要がある。D12 |
| 型付きlocalへの関数式の代入 | `local f: (number) -> number = function(x) ... end`の`f`の注釈が出ない | `local function f`への変換のため。引数・戻り値の注釈は残る |
| クラスのメソッド呼び出しの引数 | 通常の関数と同じく、`template`のない関数・メソッドの引数型は検査しない | `declare function`と`template`つき関数だけ検査する |
| クラスのメソッド本体の中の補完 | `self.`で補完が出ない(`luar complete`の結果が空) | 未対応(原因は未調査) |
| `static`メソッドだけのクラス | 未使用の`new()`と`__index`が出力される | 仕様の単純さを優先して維持(D10) |
| `luar check`が`.luard`を直接扱えない | `.luard`の診断は、それを`import type`する`.luar`経由 | |
| goto(Luau) | 関数/チャンク直下のラベルだけを扱う。入れ子のラベルと、直下のlocal宣言をまたぐラベルは診断する | Lua 5.4はネイティブ`goto` |
| `using`の制約 | `using`のある関数でgoto/ラベルが使えない。`repeat ... until`の本体直下、if式の中には書けない | D13 |
| テーブルコピー(`table.deepcopy`) | **未実装**。[language/syntax.md](language/syntax.md)に構想だけある | |
| mixin | 未実装。必要性自体を検討中 | |

## 技術的負債

1. **`.luard`のパーサーの二重化**: Rustの`modules.rs`とTypeScriptの`language.ts`。構文を足すたびに両方を直す必要がある。`luar`への問い合わせ(CLIにサブコマンドを足す)に一本化するのが望ましい。
2. **旧TypeScript frontend(`luar/`)**: `luar-vscode`が`luar/src`のlexer/parserを再利用している。拡張の索引を`luar tokens`/`luar check`だけで賄えるようにして、`luar/`を削除したい。
3. **コンパイル結果の実行テストが無い**: 生成コードの形を文字列で検査するだけで、Lua 5.4/Luauのランタイムで実行して挙動を確かめるテストは無い。`using`の`pcall`中継や、goto dispatcherのような制御フローは、実行テストがあると安全。Luau/Lua 5.4のバイナリをCIやテストに組み込むのが候補。
4. **`readme.md`の例のコード**: 説明用の断片が多く、そのままではコンパイルできないものがある。例をテストとして検査できるようにしたい。
5. **`checker.rs`が大きい**(2500行超): 型の格子、クラス検査、文の検査が1ファイルにある。`checker/annotation.rs`のように責務ごとに分割していく。`codegen.rs`(2300行超)も同様。

## 今後の候補

- クラスをLuauの型として出力する(`any`を減らす)。
- `luar check`で`.luard`を直接検査する。
- クラスのメソッド本体の中の補完(`self.`)。
- `using`をif式・`repeat`でも使えるようにする。実行テストが前提。
- エディタ: リネーム、参照の検索、シグネチャヘルプ。
- 型エラーの位置を、文全体ではなく該当のトークンに絞る(現状は文の行を示すものがある)。
