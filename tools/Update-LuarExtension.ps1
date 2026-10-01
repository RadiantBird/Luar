[CmdletBinding()]
param(
    # Also rebuild and install the compiler (luar.exe).
    [switch]$WithCompiler
)

# NOTE: keep this file ASCII-only. Windows PowerShell 5.1 reads BOM-less UTF-8 as the
# ANSI code page, and multibyte characters in comments can swallow the next line.

$ErrorActionPreference = "Stop"
$extensionId = "luar.luar-language"

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
    # A leftover .vsix could be installed by mistake, so remove old ones first.
    Get-ChildItem -Path $extensionRoot -Filter "*.vsix" | Remove-Item -Force

    # vsce asks for confirmation when repository/LICENSE/.vscodeignore are missing; answer "y".
    # (PowerShell swallows the first "--", so "npm run package -- args" does not work; call vsce directly.)
    "y", "y", "y" | & npx --yes @vscode/vsce package --no-dependencies --allow-missing-repository --skip-license
    if ($LASTEXITCODE -ne 0) {
        throw "VSIX packaging failed with exit code $LASTEXITCODE"
    }

    $vsix = Get-ChildItem -Path $extensionRoot -Filter "*.vsix" | Select-Object -First 1
    if (-not $vsix) {
        throw "Packaging finished but no .vsix was produced in $extensionRoot"
    }

    # Installing over the same version with --force does not always replace the files,
    # so uninstall first (a failure here, e.g. not installed yet, is fine).
    & code --uninstall-extension $extensionId
    Start-Sleep -Seconds 1

    # Pass the path as a single argument so spaces in it do not split it.
    & code --install-extension $vsix.FullName --force
    if ($LASTEXITCODE -ne 0) {
        throw "Extension install failed with exit code $LASTEXITCODE"
    }
}
finally {
    Pop-Location
}

# Verify the installed files are the freshly built ones.
$installedServer = Join-Path $env:USERPROFILE ".vscode\extensions\$extensionId-*\out\server.js"
$installedFile = Get-ChildItem -Path $installedServer -ErrorAction SilentlyContinue | Select-Object -First 1
$builtFile = Get-Item (Join-Path $extensionRoot "out\server.js")
if (-not $installedFile) {
    Write-Warning "Could not find the installed extension under $env:USERPROFILE\.vscode\extensions."
}
elseif ($installedFile.Length -ne $builtFile.Length) {
    Write-Warning "Installed server.js ($($installedFile.FullName)) differs from the built one. Close ALL VS Code windows and run this script again."
}

Write-Host "Installed $($vsix.Name). Fully restart VS Code (close every window) to apply it."
