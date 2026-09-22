# Luar + LÖVE: Star Catcher

画像なしで動く、落ちてくる星を受け止める小さな LÖVE ゲームです。`main.luar` は`import type colors`で補完用の`colors.luard`を読み、実体は`!include("./colors.luar")`でinline展開します。

PowerShellで、リポジトリ直下から次を実行します。

```powershell
.\tools\Update-Luar.ps1
cd .\luar-vscode\playground\love-star-catcher
luar compile --target lua54 .\main.luar .\main.lua
love .
```

操作は左右キーまたは`A`/`D`です。星を取り逃すとlifeが減り、ゲームオーバー時はEnterで再開します。

`main.lua`は生成物なので、変更する場合は`main.luar`と`colors.luar`を編集して再コンパイルしてください。
