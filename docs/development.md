# 開発者ガイド

ビルド、テスト、更新、機能の足し方、落とし穴。構造は[architecture.md](architecture.md)、設計の理由は[design-decisions.md](design-decisions.md)。

## 前提

- Windows 11 + PowerShell(スクリプトはPowerShell。Git Bashも使える)。
- Rust(edition 2024)、Node.js、(VSIX生成に)`npx`。
- このリポジトリ(`Luar Programming Language`)は、上位のRecubinエンジンのリポジトリの中にあるが、**gitは別**。コミットはこのディレクトリで行う。
- Lua / Luauのランタイムはこの環境に入っていない。生成コードを実行して確かめるには、Luau playgroundやLÖVEなど外部の実行環境を使う。テストが検査するのは、生成コードの**形**(文字列)と診断。

## ビルドと更新

| 目的 | コマンド |
|---|---|
| コンパイラをビルド | `cd luar-rs && cargo build` |
| 全テスト | `cd luar-rs && cargo test --no-fail-fast` |
| 一部のテスト | `cargo test --test using`(ファイル名) / `cargo test <名前の一部>` |
| TypeScript側のテスト | `cd luar && npx vitest run` |
| `luar.exe`を更新(PATHのもの) | `.\tools\Update-Luar.ps1`(Release版をビルドして`%USERPROFILE%\.cargo\bin`へコピー) |
| 拡張を更新(VSIX生成と再インストール) | `.\tools\Update-LuarExtension.ps1`。コンパイラも更新するなら`-WithCompiler` |
| 実行 | `cargo run -q -- compile <入力.luar>`(出力は標準出力) |

- `target/`はビルド成果物でPATHに入れない。PATHには`%USERPROFILE%\.cargo\bin`だけを入れ、更新スクリプトが置き換える実行ファイルを使う。
- 拡張はVSIXで使われるので、ソースを変えたら`Update-LuarExtension.ps1`で再インストールし、VS Codeを再読み込みする。
- エンジン(Recubin)は`python build.py build`のたびに`luar-rs`をビルドして`dlls/luar_compiler.dll`へ反映する。

### CLI

```powershell
luar compile [--target luau|lua54] <input.luar> [output]
luar check [--target ...] <input.luar>
luar check [--target ...] --stdin --source-path <path> [--diagnostic-format json]
luar dump-ir <input.luar>
luar complete   --stdin --source-path <path> --offset <utf16-offset>
luar tokens     --stdin --source-path <path>
luar definition --stdin --source-path <path> --offset <utf16-offset>
luar imports    --stdin --source-path <path>
```

`compile`の`output`を省くと標準出力。`--target`の既定は`luau`で、出力ファイルの拡張子では決まらない。`--offset`は文書先頭からのUTF-16コード単位。出力の形式は[architecture.md](architecture.md)。

## テストの構成(`luar-rs/tests`)

どのテストも、ライブラリのAPI(`compile_source_with_options`など)を呼び、出力文字列と診断を検査する。一時ディレクトリにファイルを置くヘルパー(`TempProject`)で、`!include`や`.luard`を含むケースを作る。

| ファイル | 検査するもの |
|---|---|
| `type_checking.rs` | 型検査全般(代入、演算、戻り値の推論) |
| `generics.rs` | `template`/`type`、ジェネリックな呼び出し、Lua 5.4のコメント出力 |
| `unions.rs` | ユニオン型 |
| `casts.rs` | `::`の規則と出力 |
| `luau_types.rs` | Luauターゲットの型注釈の出力、Lua 5.4で消えること |
| `stdlib.rs` | 標準ライブラリの型 |
| `oop.rs` | クラス、継承、アクセス制御、`new`、`friend` |
| `using.rs` | `using`と`free`の出力・制約 |
| `modules_const.rs` | `import type`、`.luard`、`declare class`、`const` |
| `imports.rs` | `from`パス、`!include`のパス・`return`ヒント、`luar imports` |
| `if_expressions.rs` | ブロックif式、`:=` |
| `control_flow_targets.rs` | goto/continueのターゲット別の出力 |
| `completion.rs` | 補完 |
| `navigation.rs` / `highlighting.rs` / `definitions.rs` | semantic tokens、定義ジャンプ、`declare`の読み込み |
| `cli.rs` | CLIの引数と終了コード |

`luar/tests/*.test.ts`(vitest)は旧frontendと`language.ts`(`vscode-language.test.ts`)の検査。

## 新しい構文・機能を足すチェックリスト

構文を1つ足すと、次の場所を触ることが多い。漏れると「コンパイルは通るが、エディタだけ誤動作」のような症状になる。

1. `lexer.rs`: 新しいトークンが要るか。文脈依存のキーワードは`Ident`のままにして、`parser.rs`で`is_contextual`と先読みで判別する(`using`、`const`、`type`の例)。
2. `ast.rs`: ノードを足す。既存のノードにフィールドを足すと、`..`なしのパターンが壊れる(`checker.rs`の`Local | Const`の複合パターンなど)。
3. `parser.rs`: `parse_stmt`の分岐と、文を作る関数。
4. `resolver.rs`: 名前を束縛するなら`declare`。import・未定義グローバルの扱い。
5. `checker.rs`: 型検査。`check_stmt_types`(型環境の更新)と、`check_stmt_access`相当のアクセス検査の両方。
6. `control_flow.rs`: goto/break/continue/localの数え方に影響するなら更新。
7. `codegen.rs`: Luau/Lua 5.4の出力。名前を集める関数(`collect_stmt_names`など)と、`contains_goto`/`contains_continue`の走査にも新しい文を入れる。
8. `symbols.rs`: 色分けと定義ジャンプ(字句ベース)。宣言の形を`declare_names`などに教える。
9. `include.rs`/`rename.rs`: `!include`の走査や改名に関わるなら。
10. エディタ: `luar-vscode/syntaxes/luar.tmLanguage.json`(色)、`src/language.ts`(`KEYWORDS`、`.luard`のパーサー)、`package.json`(設定・凡例)。TypeScript側の`.luard`パーサーは構文を追わせないとエラーを誤報する。
11. テスト: `luar-rs/tests`に追加し、既存テストの出力変更を確認する。
12. ドキュメント: `docs/language/`の該当章、設計の理由は`design-decisions.md`、完了したら`progress.md`。

## 落とし穴

- **改行コード**: Windowsの`core.autocrlf=true`のため、作業ツリーのファイルはCRLFで、インデックスはLF。`git apply --cached`などでパッチを作るときは、バイト列で扱う(テキストモードだと壊れる)。PythonでRustのソースを書き換えるなら、`newline=''`で読み、`\r\n`を保ったまま置換する。
- **シェルの引用符**: Bashのヒアドキュメントで、`\n`や`\\`を含むRustの文字列を書き込むと、エスケープが半分になって壊れる。Python/ツールで編集するときは、書いた後に内容を確認する。`"\\t"`のようなJSON中の正規表現も同様(`\b`が後退制御文字になった事例がある)。
- **コミットしないファイル**: `.gitignore`と`.vscode/settings.json`は、開発者が個人的に変えていることがある。依頼がなければコミットに含めない。
- **メソッド呼び出しの書き換えはアドレス依存**: `method_calls`はASTノードのアドレスで書き換え位置を特定する。チェッカーが解析したASTを、書き換えの前に作り直さないこと。
- **`validate_luau_label_layout`**: Luauのディスパッチャは、関数/チャンク直下のラベルだけを扱う。入れ子のラベルは診断する。
- **出力が変わる変更は既存テストを壊す**: コード生成を変えたら、`cargo test`で落ちたテストの期待値が「古い仕様」か「バグ」かを区別してから直す。
- **`readme.md`の例の実行**: 例のコードは必ずしもそのままコンパイルできるとは限らない(説明用の断片)。`luar-vscode/playground/`に動かせるサンプルがある。

## 動作確認用のサンプル

- `luar-vscode/playground/`: 機能別のサンプル(`classes.luar`、`include.luar`、`import-path.luar`、`raii.luar`、`goto.luar`、`recubin/`、`roblox/`、`love-star-catcher/`など)。エディタで開いて色分けや診断を確かめる。
- `luar/examples/`: 旧frontend用のサンプル。

## コミットのスタイル

- 1コミット1テーマ。日本語の件名で、何ができるようになったかを書く(例: `Luau向けの出力に型注釈をそのまま書くようにした`)。
- 複数の機能が同じファイルに混ざったときは、変更箇所(hunk)ごとに分けてステージする。分けたあとのインデックスだけを取り出して、`cargo test`が通るかを確かめる(`git checkout-index -a --prefix=<dir>/`)。
