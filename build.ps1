$ErrorActionPreference = 'Stop'
Push-Location -LiteralPath $PSScriptRoot
try {
    if (Test-Path -LiteralPath '.build/cargo/bin/cargo.exe') {
        $env:RUSTUP_HOME = Join-Path $PSScriptRoot '.build/rustup'
        $env:CARGO_HOME = Join-Path $PSScriptRoot '.build/cargo'
        $env:CARGO_TARGET_DIR = Join-Path $PSScriptRoot '.build/target'
        $env:PATH = "$env:CARGO_HOME/bin;$PSScriptRoot/.build/w64devkit/bin;$env:PATH"
        $env:LIBRARY_PATH = Join-Path $env:RUSTUP_HOME 'toolchains/stable-x86_64-pc-windows-gnu/lib/rustlib/x86_64-pc-windows-gnu/lib/self-contained'
    }
    cargo build --locked --release
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
    $metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
    $exe = Join-Path $metadata.target_directory 'release/rkn-guard-panel.exe'
    New-Item -ItemType Directory -Force dist | Out-Null
    Copy-Item -LiteralPath $exe -Destination dist/rkn-guard-panel.exe -Force
    Get-Item -LiteralPath dist/rkn-guard-panel.exe
} finally {
    Pop-Location
}

