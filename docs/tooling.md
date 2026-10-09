# ツールとエディタ連携

コンパイラの更新、VS Code拡張の更新、補完・色分け・定義ジャンプ、lintの扱い。
拡張機能の内部構造は[architecture.md](architecture.md)、ビルドとテストの手順は[development.md](development.md)を参照。

## コンパイラの更新（Windows）
開発中にコンパイラを更新するときは、リポジトリ直下から次を実行する。

```powershell
.\tools\Update-Luar.ps1
```

このスクリプトはRelease版をビルドし、`%USERPROFILE%\.cargo\bin\luar.exe`（`CARGO_HOME`を設定している場合は`%CARGO_HOME%\bin\luar.exe`）へ置き換える。更新結果は次で確認できる。

```powershell
luar --version
Get-Command luar -All
```

`luar-rs\target\release`はビルド成果物であり、PATHへ登録しない。PATHにはCargoの`bin`ディレクトリだけを登録して、常に更新スクリプトが置き換える実行ファイルを使用する。

VS Code拡張はPATH上の`luar`へ未保存の本文を渡し、Rustコンパイラと同じ診断を表示する。別の実行ファイルを使う場合は`luar.compiler.path`、互換性検査の対象は`luar.target`（`luau`または`lua54`）で設定する。コンパイラが見つからない場合もハイライトと補完は利用できるが、意味診断は無効になる。

### 補完
`.`(`:`)を打つと、VS Codeは`luar complete`でコンパイラのチェッカーへ問い合わせ、レシーバの型に応じたメンバーを種別(field/method/function)と型付きで提案する。識別子の補完は、その位置から見えるlocal・const・関数・クラス・モジュールを、種別(`local x: number`、`const NAME: string`など)と型付きで返す。外部から見えるのはpublicメンバーだけで、クラス自身(`Dog.`)にはstaticメソッドと`new`、インスタンス(`dog.`)にはフィールドとインスタンスメソッドが出る。

```powershell
luar complete --stdin --source-path main.luar --offset 123 < main.luar
```

`--offset`は文書先頭からのUTF-16コード単位のオフセットで、結果は`{"items":[{"label","kind","type","detail"}]}`のJSONである。入力途中で閉じ括弧が足りない場合も、補った版で解析する。コンパイラを呼べない・結果が空のときは、従来のこのファイル内の索引へフォールバックする。クラスのメソッド本体の中(`self.`など)の補完は未対応である。

### 色分け(semantic tokens)と定義ジャンプ
キーワードや文字列などはTextMate文法で、変数・const・引数・関数・メソッド・フィールド・クラス・モジュール(`import type`と`!include`の束縛名)・型名は、コンパイラの解析結果で色分けする(`luar tokens`)。constと宣言位置には`readonly`/`declaration`、staticメソッドには`static`の修飾が付く。`.luard`のクラス名もクラスとして色付けされる。拡張機能は`[luar]`/`[luard]`でsemantic highlightingを既定で有効にする。

Ctrlクリック(F12)の定義ジャンプは`luar definition`で行う。対象は次のとおり。

- 同じファイルの`local`/`const`/関数/引数/クラス/メソッド/フィールドの宣言(スコープとシャドーイングを考慮)。
- `!include`したファイルの宣言。`clsdef.dog.name`の`dog`/`name`は、レシーバの型からインクルード先の定義へ飛ぶ。
- `import type`した`.luard`の`declare`/`declare class`とそのメンバー。`import type`の名前は`.luard`自体を開く。
- `!include("./x.luar")`のパス文字列と、その束縛名は`x.luar`を開く。

```powershell
luar tokens --stdin --source-path main.luar < main.luar
luar definition --stdin --source-path main.luar --offset 123 < main.luar
```

型が決まらないメンバー参照は、同名のメンバー宣言を全て候補として返す。色分けと定義ジャンプは字句解析ができれば動くため、入力途中の構文エラーがあるファイルでも使える。

拡張機能をVSIXから利用している場合、ソース変更後はビルドだけでなくVSIXの再生成と再インストールが必要になる。次のスクリプトがVSIXの生成からインストールまでを行う。`-WithCompiler`を付けると`luar.exe`の更新も同時に行う。実行後はVS Codeを再読み込みする。

```powershell
.\tools\Update-LuarExtension.ps1
.\tools\Update-LuarExtension.ps1 -WithCompiler
```

```powershell
cd .\luar-vscode
npm run package
code --install-extension .\luar-language-0.1.0.vsix --force
```

## lintと外部runtime

構文エラーと未定義globalの診断は、原因となるtoken全体を赤線または黄線で示す。未定義globalはLuaのruntime依存値を扱えるようwarningであり、`luar check`の終了コードを失敗にしない。Lua/Luau標準globalはあらかじめ認識する。

外部runtimeは設定項目ではなく、通常の`.luard`で宣言して使い回す。

```lua
-- love.luard
declare global love: Love

-- main.luar
import type love
love.graphics.print("hello")
```

`import type love`は宣言だけを読み、`require`などの実行時コードを生成しない。未宣言の`love`や綴り誤りの`lovve`はwarningになる。
