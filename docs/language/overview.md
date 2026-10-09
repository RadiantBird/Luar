# 言語の概要

LuarはLuau(Lua)から派生した言語で、Luau/Lua 5.4のソースへコンパイルされる。ほぼLuauのスーパーセットで、クラス、静的な型検査、ジェネリクス、モジュールの型宣言などを足している。

## 特徴

- **クラス**: `class X is public is ... end end`。継承(`is Parent`)、`abstract`/`override`/`final`、public/private、`friend`、演算子オーバーロード、デストラクタ`free`と、スコープ脱出で自動解放する`using`。→ [classes.md](classes.md)
- **型**: 注釈、`type`、`template <T>`、ユニオン、キャスト`::`、標準ライブラリの型つき宣言、戻り値の推論。→ [types.md](types.md)
- **構文の追加**: ブロックif式、条件付きbinding `:=`、`goto`/`continue`、`const`、バッククォートのフォーマット文字列。→ [syntax.md](syntax.md)
- **モジュール**: 型だけを読む`import type`、ソースを取り込む`!include`、宣言ファイル`.luard`。→ [modules.md](modules.md)
- **エディタ**: 診断・補完・色分け・定義ジャンプ。→ [../tooling.md](../tooling.md)

## 拡張子

| 拡張子 | 内容 |
|---|---|
| `.luar` | Luarのソース |
| `.luard` | 宣言ファイル(`declare` / `declare class` / `type`)。実行時コードは生成しない |
| `.luau` / `.lua` | 出力。`!include`では`.lua`(Lua 5.4)も取り込める |

## コンパイルターゲット

LuarはLuauとLua 5.4へ出力できる。既定は後方互換のため`luau`で、出力ファイルの拡張子からはターゲットを決めない。

```powershell
luar compile --target luau main.luar main.luau
luar compile --target lua54 main.luar main.lua
luar check --target lua54 main.luar
```

| 違い | Luau | Lua 5.4 |
|---|---|---|
| 型注釈 | Luauの型構文で出力 | 消去し、元の宣言を`--`コメントで残す |
| `const` | `const`(Luau 0.731) | `local x <const>` |
| `continue` | ネイティブ | `repeat ... until true`と`break`フラグ |
| `goto` | 直下のラベルをディスパッチャに変換 | ネイティブ |
| `using` | `pcall`で包んで`free()` | `local x <close>` |
| 標準ライブラリの型 | Luau(`task`、`bit32`など) | Lua 5.4(`io`、`utf8`など) |

## コンパイラの構造

構造と処理の流れは[../architecture.md](../architecture.md)、設計の理由は[../design-decisions.md](../design-decisions.md)にある。生成コードは、字句解析→構文解析→名前解決→型検査→コード生成の順で作られる。

> 以前のreadmeには「HIR→CFIR→backend」という処理フローがあったが、コード生成はASTから直接行う。CFIRは`luar dump-ir`と制御フローの検証のためだけに使う。
