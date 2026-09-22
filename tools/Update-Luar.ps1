[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $repositoryRoot "luar-rs\Cargo.toml"
if (-not (Test-Path $manifestPath)) {
    throw "Luar Cargo manifest was not found: $manifestPath"
}

& cargo build --manifest-path $manifestPath --release
if ($LASTEXITCODE -ne 0) {
    throw "Luar build failed with exit code $LASTEXITCODE"
}

$artifactPath = Join-Path $repositoryRoot "luar-rs\target\release\luar.exe"
if (-not (Test-Path $artifactPath)) {
    throw "Luar build completed but no executable was produced: $artifactPath"
}

$cargoRoot = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
$installDirectory = Join-Path $cargoRoot "bin"
$installedPath = Join-Path $installDirectory "luar.exe"

New-Item -ItemType Directory -Path $installDirectory -Force | Out-Null
Copy-Item -Path $artifactPath -Destination $installedPath -Force

Write-Host "Updated $installedPath"
& $installedPath --version
if ($LASTEXITCODE -ne 0) {
    throw "The updated Luar executable could not be started"
}

$resolvedCommand = Get-Command luar -CommandType Application -ErrorAction SilentlyContinue
if ($resolvedCommand -and $resolvedCommand.Source -ne $installedPath) {
    Write-Warning "'luar' currently resolves to $($resolvedCommand.Source), not $installedPath. Remove luar-rs\\target\\release from PATH and open a new terminal."
}
