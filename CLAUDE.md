# CLAUDE.md — Luar Programming Language

Luar(Luau/Lua 5.4へコンパイルする言語)のサブプロジェクト用の指示。上位ディレクトリのC++エンジン用`CLAUDE.md`のうち、言語・ビルド・コーディング規約(C++/CMake)に関するものは、このサブプロジェクトには適用しない。**スコープ厳守・実装前に確認・説明は最小限**の方針は共通。

## セッション開始時(必須)

1. [docs/progress.md](docs/progress.md)の一番下(最新)のセッションを読む。
2. [docs/roadmap.md](docs/roadmap.md)の既知の制限と未解決事項を確認する。
3. 依頼に関わる設計判断が[docs/design-decisions.md](docs/design-decisions.md)にあれば読む(既に決まったことを蒸し返さない)。
4. そのうえで、今回の依頼を進める。

## 構成(詳細は[docs/architecture.md](docs/architecture.md))

- `luar-rs/` — Rustのコンパイラ(**唯一の正式な実装**)。CLI `luar`、ライブラリ、エンジン向けのcdylib。
- `luar-vscode/` — VS Code拡張。意味の判断は`luar` CLIに問い合わせる。
- `luar/` — 旧TypeScript frontend。非推奨。拡張が一部(lexer/parser)を再利用しているだけ。ここに新機能を足さない。
- `docs/` — ドキュメント。`readme.md`は入口。

## 進め方

- **返答は日本語で簡潔に**。実装後の説明は最小限。
- **大きな変更は、実装の前に計画を立てて確認する**。変更するファイルと箇所を先に挙げる。仕様が曖昧なら「たぶんこうだろう」で進めず、質問する。選択肢があるときは推奨を先頭にして示す。
- **スコープを守る**。依頼された範囲だけ変える。「ついでの整理・改善」はしない。気づいた別件は、実装せず報告する。
- 既に決まった設計判断を蒸し返さない。変える必要があるときは、理由を示して確認する。
- 共通のコーディング規約は、周囲のコードに合わせる(コメントの量・命名・書き方)。コメントは日本語で、「なぜ」を書く。

## 変更のたびに守ること

1. **テストを書く**: `luar-rs/tests/`に追加する。出力の形(文字列)と診断を検査する。新しい構文は、Luau/Lua 5.4の両方のターゲットで。
2. **全テストを通す**: `cd luar-rs && cargo test --no-fail-fast`。`luar/`に触れたら`cd luar && npx vitest run`も。落ちたテストは、期待値が「古い仕様」なのか「バグ」なのかを区別してから直す。
3. **検査を省略しない**: `grep`で失敗数だけを数えると、テストファイルのコンパイルエラーを見落とす。`test result: ok`の数と`FAILED`の両方を見る。
4. **ドキュメントを同じ変更で直す**: 仕様は`docs/language/`の該当章、設計の理由は`docs/design-decisions.md`、作業の記録は`docs/progress.md`の末尾。実装と食い違う記述を見つけたら、実装を確かめて文書を直す。
5. **構文を足すときは**[docs/development.md](docs/development.md)の「新しい構文・機能を足すチェックリスト」に沿う。エディタ側(TextMate文法、`language.ts`のキーワードと`.luard`パーサー、`package.json`)の追随を忘れない。

## この環境の注意

- Windows 11、PowerShellが主、Git Bashも使える。Lua/Luauのランタイムは無い。生成コードの**実行**確認は、利用者がLuau playgroundなどで行う。実行していないことは、実行していないと伝える。
- `core.autocrlf=true`のため、作業ツリーはCRLF、インデックスはLF。ファイルをスクリプトで書き換えるときは改行を保つ。`git apply`用のパッチはバイト列で扱う。
- Bashのヒアドキュメントは`\n`・`\\`・引用符を壊すことがある。Rustの文字列やJSONの正規表現を書き込むときは、編集ツールかファイルを使い、書いた後に内容を確認する。
- 一時ファイルは、セッションのスクラッチ用ディレクトリに置く。リポジトリの`target/`などに探査用ファイルを置かない。

## コミット

- 依頼されたときだけコミットする。1コミット1テーマ、日本語の件名。
- **`.gitignore`と`.vscode/settings.json`は、開発者の個人的な変更なので、依頼がない限りコミットしない**。
- 複数のテーマが同じファイルに混ざったら、変更箇所(hunk)ごとに分けてステージし、分けたあとの状態でテストが通ることを確かめる。

## コンパイラ・拡張の反映

- コンパイラ: `.\tools\Update-Luar.ps1`(PATH上の`luar.exe`を置き換える)。
- 拡張: `.\tools\Update-LuarExtension.ps1 [-WithCompiler]`(VSIXを再生成して再インストール。実行後はVS Codeを再読み込み)。
- 変更を終えたら、利用者にこれらの実行が必要かを伝える。

## 参照

- 言語仕様: [docs/language/](docs/language/)
- エンジンとの結線: `../doc/Core/LuarCompiler.md`、`../spec.md`(ScriptExtensionとパッケージング)
- サンプル: `luar-vscode/playground/`
