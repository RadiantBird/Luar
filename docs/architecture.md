# アーキテクチャ

Luarコンパイラ(`luar-rs`)、VS Code拡張(`luar-vscode`)、旧TypeScript frontend(`luar`)の構造と、データの流れ。
言語仕様そのものは[language/](language/)、設計の理由は[design-decisions.md](design-decisions.md)を参照。

## 全体像

```
                         ┌──────────────────────────────┐
  .luar / .luard ──────▶ │ luar-rs (Rust, 唯一の正式な実装) │
                         │  ライブラリ(rlib / cdylib) + CLI(luar) │
                         └───────┬──────────────┬────────┘
                                 │              │
                 C ABI (DLL)     │              │  CLI + JSON (標準入出力)
                                 ▼              ▼
              Recubinエンジン(C++)       VS Code拡張 (luar-vscode)
              src/Core/LuarCompiler.cpp   LSPサーバー server.ts が `luar` を起動
```

| ディレクトリ | 役割 | 状態 |
|---|---|---|
| `luar-rs/` | コンパイラ本体。字句解析〜型検査〜コード生成、エディタ向けの問い合わせ(補完・色分け・定義ジャンプ・import一覧) | 正式 |
| `luar-vscode/` | VS Code拡張。TextMate文法、スニペット、LSPサーバー。意味の判断は`luar` CLIに問い合わせる | 正式 |
| `luar/` | TypeScript製の旧frontend。CLI・codegenとしては非推奨。`luar-vscode`が`luar/src`のlexer/parser/ASTを再利用している(下記「TypeScript側の重複」) | 縮小中 |
| `luau/` | Luau本体のソース(参照用)。ビルドには使わない | 参照のみ |
| `tools/` | Windows向け更新スクリプト | - |

エンジンとの結線は、エンジン側の[doc/Core/LuarCompiler.md](../../doc/Core/LuarCompiler.md)に書いてある。`luar_compiler.dll`は`python build.py`が`luar-rs`をビルドして`dlls/`へ反映する。

## コンパイルの流れ(`luar-rs`)

入口は`src/lib.rs`の`compile_source_with_options`(Luarソース→Luau/Lua 5.4ソース)と`analyze_source_with_options`(診断だけ)。

```
ソース文字列
 │ include::expand_source        !include を展開。行ごとの出典(ExpandedSource.origins)を持つ
 │ parser::Parser               トークン列 → AST (ast::Program)。lexer::Lexer が字句解析
 │ lib::prepare_analysis        import type を数え、.luard を読み込む(modules)。壊れていても読めた分は使う
 │ resolver::Resolver           名前の解決。importしたモジュールの未修飾名を mod.member へ書き換え、
 │                              未定義グローバルを警告にする
 │ checker::Checker             型検査・クラス規則の検査。メソッド呼び出しの位置も記録する
 │ method_calls::rewrite        インスタンスメソッドの `obj.m()` を MethodCall (`obj:m()`) へ書き換え
 │ control_flow::validate       goto/ラベル/break/continue/using の規則を検査
 │ validate_luau_label_layout   Luauターゲットでラベルの位置が表せるかを検査
 ▼
Analysis { program, diagnostics }   エラーが1つでもあれば Err(Vec<Diagnostic>)
 │ codegen::Codegen::for_target   AST → Luau または Lua 5.4 のソース
 ▼
出力ソース
```

- **コード生成はASTから直接行う**。`control_flow.rs`にはCFIR(基本ブロックのIR)があるが、`dump-ir`と検証のためだけに使い、codegenは通らない。以前のreadmeにあった「HIR→CFIR→backend」という処理フローは実態と異なる。
- 診断の位置は、`!include`で行がずれる前の元ファイルへ`ExpandedSource::remap_span`で戻して報告する。
- `Target`(`luau` / `lua54`)は、標準ライブラリの定義、グローバル名、コード生成の分岐に効く。拡張子からは決めない。

## モジュール一覧(`luar-rs/src`)

| ファイル | 役割 |
|---|---|
| `lexer.rs` | 字句解析。トークンは`line`/`column`(UTF-16、1始まり)を持つ。`const`/`type`/`template`/`using`/`friend`/`from`などは文脈依存で、トークン種別は`Ident` |
| `ast.rs` | AST。`TypeExpr`(型式)、`Expr`、`Stmt`、`ClassDecl`など。`Stmt::Const`は`using`も表す(`is_using`) |
| `parser.rs` | 再帰下降パーサー。`parse_stmt`が文頭の形で分岐する。`template <T>`は次の宣言1つに付く |
| `include.rs` | `!include`のインライン展開、`return <名前>`の除去、名前衝突時の改名、`.lua`の検証(`full_moon`)。展開後の行→元ファイルの対応表を返す |
| `rename.rs` | `!include`の名前衝突を解くための、トークン単位の改名 |
| `modules.rs` | `.luard`(定義ファイル)の読み込み。`ModuleDefinition`(メンバー・global・クラス・関数・型)を作る。`*_lossy`は壊れた宣言を飛ばして読み進める。`resolve_definition_path`が`import type`の`from`パスを解決 |
| `resolver.rs` | 名前解決。importしたモジュールの未修飾名の修飾、曖昧参照のエラー、未定義グローバルのwarning |
| `checker.rs` / `checker/annotation.rs` | 型検査。`ValueType`(型の格子)、クラス情報、型注釈の解決、`template`関数の呼び出し検査 |
| `stdlib.rs` | Lua 5.4 / Luau標準ライブラリの型つき宣言(`.luard`と同じ文法のテキスト) |
| `method_calls.rs` | チェッカーが記録した呼び出し位置を、メソッド呼び出し(`:`)へ書き換える |
| `control_flow.rs` | goto/ラベル/break/continue/usingの規則の検査。`dump-ir`用のCFIR |
| `codegen.rs` | AST→Luau/Lua 5.4。クラス・usingのlowering・goto dispatcher・型注釈の出力 |
| `luau_types.rs` | Luauターゲットで出力する型の文字列化(宣言の無い名前は`any`) |
| `type_comment.rs` | Lua 5.4ターゲットで、消去した型を`--`コメントに残す |
| `symbols.rs` | **字句ベース**のスコープ解析。構文エラーのあるファイルでも動く。色分けと定義ジャンプの土台 |
| `navigation.rs` | semantic tokensと定義ジャンプ。`symbols`の結果を`!include`の出典に戻し、メンバー参照だけチェッカーに問い合わせる |
| `completion.rs` | 補完。カーソル位置の式を`__luar_probe(<recv>)`/`__luar_scope()`という呼び出しに書き換えて通常の解析に通し、チェッカーがその時点の型環境を記録する |
| `lib.rs` | 公開API、`prepare_analysis`、`list_imports`、C ABI(`luar_compile*`)、診断型 |
| `main.rs` | CLI(`luar`)。サブコマンドごとにJSONを標準出力へ書く |

## チェッカー(`checker.rs`)

型の格子`ValueType`:

`Unknown` / `Nil` / `Boolean` / `Number` / `String` / `Table` / `Shape`(形の分かるテーブル) / `Function` / `FunctionSig`(`template`関数と`declare function`) / `Generic` / `Record`(注釈由来の構造的テーブル) / `Class(名前, 型引数)` / `Optional` / `Union`

- `Unknown`は「判断できない」で、外部runtime由来の値を推論だけで拒否しないための逃げ道。`is_assignable`は`Unknown`を常に許す。
- 型注釈のない関数の戻り値は、すべての`return`から推論する(`return 1`と`return nil`が混在すれば`number?`)。
- `template`つき関数の呼び出しだけ、引数の型から型引数を推論し直して検査する。通常の関数の引数は検査しない。
- クラス情報`ClassInfo`はメソッド・フィールド・親・friendを持つ。overrideの規則、abstract、finalの検査はここで行う。
- `check_stmt_types`が文ごとに型環境(`env`)を更新する。補完は`__luar_probe`への呼び出しに到達した時点の`env`を記録する。
- メソッド呼び出しの位置(ノードのアドレス)を`method_calls`が使う。**アドレスは比較するだけで、参照はしない**。ASTを再構築する処理を挟むとアドレスが変わって書き換えが失われるので注意。

## コード生成(`codegen.rs`)

- 文は`emit_stmt`が1つずつ出力し、`emit_block`が文の並び(`using`の分割を含む)を出力する。式は`lower_expr`が`LoweredExpr { prelude, expr }`にし、`prelude`は式の前に出す文。
- クラスは`local Name = {}`と`Name.__index = Name`、`function Name.new()`、`function Name.method(self, ...)`の形。privateメソッドは`local function`。演算子は`Name.__eq = function(self, other)`などのメタメソッド。
- `free`を持つクラスは`Name.__free`(本体と親の`__free`の連鎖)と`Name.free`(解放済みなら何もしない入口)に分ける。Lua 5.4ターゲットでは`Name.__close`も出す。
- `using`: Luauは`using`以降の文を`pcall(function() ... end)`で包み、`return`/`break`/`continue`をクロージャの戻り値の先頭の印(1/2/3)で外へ中継する。`...`を使うときはクロージャへ渡す。関数の境界(`emit_function_body`)で`using_scopes`を空にする。Lua 5.4は`local x <close> = ...`。
- `goto`: Lua 5.4はネイティブ。Luauは関数/チャンク直下のラベルをディスパッチャ(`while pc ~= nil do if pc == "label" then ...`)に変換する。`continue`はLuauはネイティブ、Lua 5.4は`repeat ... until true`と`break`フラグで表す。
- 型: Luauターゲットは型注釈を`signature()`/`type_text()`で出力する。Lua 5.4ターゲットは`type_comment`でコメントにする。`self.pending_type_params`と`scope_type_params`は、関数の型引数を本体の注釈から参照するための仕組み(`push_scope`/`pop_scope`と対)。
- 名前の生成は`generated_name`(`__luar_<用途>_<連番>`)。ユーザーの名前と衝突しないよう、`reserved_names`(プログラム中の全名)を避ける。

## `!include`と行の対応(`include.rs`)

`expand_source`は、`local|const 名前 = !include("相対パス")`の行を、取り込んだファイルの本文に置き換える。末尾の`return <名前>`は取り除き、代入先と名前が違うときだけ別名を束縛する。ネストした`!include`は、そのファイルからの相対パス。

- 結果は`ExpandedSource { source, origins }`で、`origins`は展開後の各行の(ファイル, 行)。診断・色分け・定義ジャンプは必ずこの表で元の位置に戻す。
- 名前の衝突は`rename.rs`でinclude側を`<名前>__<束縛名>`に改名する。
- `.lua`は`full_moon`でLua 5.4として構文検証する。

## `.luard`と`import type`(`modules.rs`)

- `import type name [from "相対パス.luard"]`は、`name.luard`(または`from`のパス)の宣言だけを読む。実行時コードは出さない。
- `ModuleDefinition`はメンバー名・global名・型・クラス・関数・`type`宣言を持つ。リゾルバが未修飾名の修飾に、チェッカーが型に使う。
- 壊れた宣言は`*_lossy`が飛ばして読み進める。読み込みに失敗した定義がある間は`imports_incomplete`を立て、リゾルバが未定義グローバルの警告を止める(エラーの連鎖を防ぐ)。
- `!include`した名前と同名の`import type`は、`.luard`が無ければincludeしたソースのトップレベルから定義を作る。
- パスの規則(相対のみ、`.luard`のみ、絶対パス不可)は`resolve_definition_path`に集約している。エディタは`luar imports`でこの結果を受け取る。

## エディタ向けの問い合わせ(CLI)

`luar-vscode`の`server.ts`は、未保存の本文を標準入力で`luar`に渡す。

| コマンド | 出力 | 用途 |
|---|---|---|
| `luar check --stdin --source-path <p> --diagnostic-format json` | `{"diagnostics":[{file,line,column,endLine,endColumn,severity,message}]}` | 診断 |
| `luar complete ... --offset <UTF-16>` | `{"items":[{label,kind,type,detail}]}` | 補完 |
| `luar tokens ...` | `{"tokens":[{line,column,length,type,modifiers}]}`(0始まり) | semantic tokens |
| `luar definition ... --offset <UTF-16>` | `{"locations":[...]}` | 定義ジャンプ |
| `luar imports ...` | `{"imports":[{name,line,column,path,error}]}` | `import type`の`.luard`の場所 |
| `luar compile [--target] <in> [out]` | 出力ソース | コンパイル |
| `luar dump-ir <in>` | CFIRのテキスト | 調査用(形式は非公開) |

`tokens`と`definition`は字句解析ができれば動く(`symbols.rs`が字句ベースのため)。入力途中の構文エラーがあるファイルでも色分けと定義ジャンプが効くのは、この設計による。

## C ABI(エンジン連携)

`lib.rs`が`luar_compile`、`luar_compile_with_path`、`luar_compile_with_target`、`luar_compile_target`、`luar_compile_with_path_target`、`luar_get_errors`を`extern "C"`で公開する(cdylib)。`!include`と`import type`を解決するには`*_with_path`系を使う。エラー文字列はスレッドローカルのバッファに入り、`luar_get_errors`で取り出す。

## VS Code拡張(`luar-vscode`)

- `syntaxes/luar.tmLanguage.json`: TextMate文法。キーワード・文字列・テンプレート文字列の式・型注釈など、意味を要しない色分け。コンパイラのsemantic tokensがあれば後から上書きされる。
- `src/extension.ts`: クライアント。言語サーバーを起動する。
- `src/server.ts`: LSPサーバー。診断(`luar check`)、補完、定義ジャンプ、ホバー、ドキュメントシンボルを受け持つ。`luar`が見つからない/結果が空のときは、`language.ts`の索引にフォールバックする。
- `src/language.ts`: コンパイラなしで動く寛容な索引(アウトライン・補完の簡易版)と、`.luard`のTypeScript版パーサー。
- `package.json`: 設定(`luar.compiler.path`、`luar.target`)、semantic tokensの凡例(`defaultLibrary`など)と`semanticTokenScopes`、`[luar]`/`[luard]`の`configurationDefaults`。

### TypeScript側の重複(既知の負債)

`.luard`のパーサーが、Rust(`modules.rs`)とTypeScript(`language.ts`の`parseModuleDefinition`)に二重にある。Rustの構文を増やしたら、TypeScript側も追随させないと、エディタが誤ったエラーを出す(「expected 'declare'」という誤報がその例)。いずれ`luar`への問い合わせに一本化する予定。[roadmap.md](roadmap.md)参照。

## テストの構成

`luar-rs/tests/*.rs`が統合テスト(ライブラリのAPI経由でコンパイル結果と診断を検査)。`luar/tests/*.test.ts`(vitest)は旧frontendと、拡張が使う`language.ts`の検査。詳細は[development.md](development.md)。
