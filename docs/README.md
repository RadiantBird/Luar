# Luar ドキュメント

目的に応じて読む文書を選ぶ。

## 言語を使う人

| 文書 | 内容 |
|---|---|
| [language/overview.md](language/overview.md) | 言語の概要、拡張子、コンパイルターゲット |
| [language/types.md](language/types.md) | 型注釈、`type`/`template`、ユニオン、キャスト、推論、標準ライブラリ |
| [language/syntax.md](language/syntax.md) | ブロックif式、`:=`、goto/continue、フォーマット文字列、const |
| [language/classes.md](language/classes.md) | クラス、継承、アクセス制御、`free`、`using` |
| [language/modules.md](language/modules.md) | `import type`、`!include`、`.luard`、`declare class` |
| [tooling.md](tooling.md) | コンパイラ・拡張の更新、補完、色分け、定義ジャンプ、lint |

## 開発する人

| 文書 | 内容 |
|---|---|
| [architecture.md](architecture.md) | コンパイラとエディタ拡張の構造、処理の流れ、モジュールの役割 |
| [design-decisions.md](design-decisions.md) | 「なぜそうしたか」と、見送った案 |
| [development.md](development.md) | ビルド、テスト、機能の足し方(チェックリスト)、落とし穴 |
| [roadmap.md](roadmap.md) | 既知の制限、技術的負債、今後の候補 |
| [progress.md](progress.md) | 開発進捗ログ(最新が一番下) |
| [../CLAUDE.md](../CLAUDE.md) | AIエージェント向けのプロジェクト指示 |

## 読む順番の例

- **初めて触る**: overview → 興味のある章 → tooling
- **コンパイラを直す**: architecture → development(チェックリスト) → design-decisions → 該当のソース
- **機能を足す**: design-decisions(関連する判断) → development(チェックリスト) → roadmap(重なる制限) → progress(最新の状況)

## ドキュメントの運用

- 仕様を変えたら、`language/`の該当章を同じ変更で直す。
- 設計上の判断(特に、見送った案があるもの)は`design-decisions.md`に残す。
- 作業を終えたら`progress.md`の末尾に追記する。
- 実装と食い違う記述を見つけたら、実装を確かめて文書を直す(以前のreadmeには、未実装の`table.deepcopy`や実態と異なる処理フローが書かれていた)。
