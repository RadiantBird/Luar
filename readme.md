# Luar Programming Language

Luau(Lua)から派生した言語。クラス、静的な型検査、ジェネリクス、モジュールの型宣言などを足して、Luau/Lua 5.4へコンパイルする。

```luau
class Counter is
    public is
        value = 0

        static function new(start: number)
            self.value = start
        end

        function add(amount: number): number
            self.value = self.value + amount
            return self.value
        end

        function free() -- デストラクタ
            print("closed")
        end
    end
end

using c = Counter.new(1) -- スコープを抜けると c.free() が自動で呼ばれる
print(c.add(2))          -- Luauへは c:add(2) として出力される
```

## 構成

| ディレクトリ | 内容 |
|---|---|
| `luar-rs/` | コンパイラ本体(Rust)。CLI `luar` とライブラリ(エンジン向けにDLLも出力) |
| `luar-vscode/` | VS Code拡張(文法・LSP)。意味の判断は`luar`に問い合わせる |
| `luar/` | 旧TypeScript frontend(非推奨。拡張が一部を再利用) |
| `luau/` | Luau本体のソース(参照用) |
| `tools/` | 更新スクリプト(Windows) |
| `docs/` | ドキュメント |

## クイックスタート

```powershell
# コンパイラをビルドして使う
cd luar-rs
cargo run -q -- compile ..\luar-vscode\playground\classes.luar

# テスト
cargo test --no-fail-fast

# luar.exe を更新(PATH上のもの) / VS Code拡張を更新
.\tools\Update-Luar.ps1
.\tools\Update-LuarExtension.ps1 -WithCompiler
```

## ドキュメント

[docs/README.md](docs/README.md)に目次がある。

- 言語: [概要](docs/language/overview.md) / [型](docs/language/types.md) / [構文](docs/language/syntax.md) / [クラス](docs/language/classes.md) / [モジュール](docs/language/modules.md)
- ツール: [コンパイラ・拡張の更新、補完、色分け](docs/tooling.md)
- 開発: [アーキテクチャ](docs/architecture.md) / [設計判断](docs/design-decisions.md) / [開発者ガイド](docs/development.md) / [ロードマップ](docs/roadmap.md) / [進捗ログ](docs/progress.md)
- AIエージェント向け: [CLAUDE.md](CLAUDE.md)
