$ErrorActionPreference = "Stop"

$TargetTriple = "x86_64-unknown-linux-gnu.2.17"
$FallbackTargetTriple = "x86_64-unknown-linux-gnu"
$OutputDir = ".\target\glibc217"
$BinaryName = "el"
$OutputBinary = Join-Path $OutputDir $BinaryName
$env:ZIG_LOCAL_CACHE_DIR = (Resolve-Path ".\target").Path + "\zig-local-cache"
$env:ZIG_GLOBAL_CACHE_DIR = (Resolve-Path ".\target").Path + "\zig-global-cache"

Write-Host "=== Building el for Linux x86_64 glibc 2.17 ===" -ForegroundColor Green

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

Write-Host "Running cargo zigbuild..." -ForegroundColor Yellow
cargo zigbuild --release --target $TargetTriple

if ($LASTEXITCODE -ne 0) {
    Write-Host "cargo zigbuild failed!" -ForegroundColor Red
    exit 1
}

if (Test-Path ".\target\$TargetTriple\release\$BinaryName") {
    $BuiltBinary = ".\target\$TargetTriple\release\$BinaryName"
} elseif (Test-Path ".\target\$FallbackTargetTriple\release\$BinaryName") {
    $BuiltBinary = ".\target\$FallbackTargetTriple\release\$BinaryName"
} else {
    Write-Host "Built binary not found under target\$TargetTriple or target\$FallbackTargetTriple" -ForegroundColor Red
    exit 1
}

Copy-Item -Force $BuiltBinary $OutputBinary

Write-Host "Compressing binary with upx..." -ForegroundColor Yellow
upx --best --lzma $OutputBinary

if ($LASTEXITCODE -ne 0) {
    Write-Host "upx compression failed!" -ForegroundColor Red
    exit 1
}

$BinaryInfo = Get-Item $OutputBinary

Write-Host ""
Write-Host "=== Build completed successfully ===" -ForegroundColor Green
Write-Host "Binary location: $OutputBinary" -ForegroundColor Cyan
Write-Host ("Compressed size: {0:N0} bytes" -f $BinaryInfo.Length) -ForegroundColor Cyan
