[CmdletBinding()]
param(
    # コンパイラ(luar.exe)も同時に更新する
    [switch]$WithCompiler
)

$ErrorActionPreference = "Stop"

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$extensionRoot = Join-Path $repositoryRoot "luar-vscode"
if (-not (Test-Path (Join-Path $extensionRoot "package.json"))) {
    throw "VS Code extension was not found: $extensionRoot"
}

if ($WithCompiler) {
    & (Join-Path $PSScriptRoot "Update-Luar.ps1")
}

if (-not (Get-Command code -ErrorAction SilentlyContinue)) {
    throw "'code' command was not found. Enable 'Shell Command: Install code command in PATH' in VS Code."
}

Push-Location $extensionRoot
try {
    # 古いVSIXが残っていると別のファイルを入れてしまうので、先に消す。
    Get-ChildItem -Path $extensionRoot -Filter "*.vsix" | Remove-Item -Force

    & npm run package
    if ($LASTEXITCODE -ne 0) {
        throw "VSIX packaging failed with exit code $LASTEXITCODE"
    }

    $vsix = Get-ChildItem -Path $extensionRoot -Filter "*.vsix" | Select-Object -First 1
    if (-not $vsix) {
        throw "Packaging finished but no .vsix was produced in $extensionRoot"
    }

    # パスに空白があっても分割されないよう、引数として1つで渡す。
    & code --install-extension $vsix.FullName --force
    if ($LASTEXITCODE -ne 0) {
        throw "Extension install failed with exit code $LASTEXITCODE"
    }
}
finally {
    Pop-Location
}

Write-Host "Installed $($vsix.Name). Reload VS Code (Ctrl+Shift+P > Developer: Reload Window) to apply it."
