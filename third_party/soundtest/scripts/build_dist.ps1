param(
  [string]$Target = "",
  [ValidateSet("release", "dev")]
  [string]$Profile = "release",
  [string]$ModelDir = "",
  [string]$OutDir = "",
  [switch]$NoBootstrap,
  [switch]$Archive
)

$ErrorActionPreference = "Stop"

$root = Resolve-Path (Join-Path $PSScriptRoot "..")

if ([string]::IsNullOrWhiteSpace($ModelDir)) {
  $ModelDir = Join-Path $root "models\\chatterbox-multilingual-onnx"
}
if ([string]::IsNullOrWhiteSpace($OutDir)) {
  $OutDir = Join-Path $root "dist"
}

if (-not (Test-Path -LiteralPath $ModelDir -PathType Container)) {
  if (-not $NoBootstrap) {
    Write-Host "note: model dir missing: $ModelDir"
    Write-Host "      bootstrapping assets..."
    & (Join-Path $root "scripts\\bootstrap_chatterbox_multilingual_onnx.ps1") -ModelDir $ModelDir
  } else {
    throw "model dir missing: $ModelDir`nRun: powershell -ExecutionPolicy Bypass -File scripts\\bootstrap_chatterbox_multilingual_onnx.ps1 -ModelDir `"$ModelDir`""
  }
}

$requiredFiles = @(
  "tokenizer.json",
  "default_voice.wav",
  "onnx/speech_encoder.onnx",
  "onnx/speech_encoder.onnx_data",
  "onnx/embed_tokens.onnx",
  "onnx/embed_tokens.onnx_data",
  "onnx/conditional_decoder.onnx",
  "onnx/conditional_decoder.onnx_data",
  "onnx/language_model_q4.onnx",
  "onnx/language_model_q4.onnx_data"
)

$missing = @()
foreach ($rel in $requiredFiles) {
  $p = Join-Path $ModelDir $rel
  if (-not (Test-Path -LiteralPath $p -PathType Leaf)) {
    $missing += $p
    continue
  }
  try {
    if ((Get-Item -LiteralPath $p).Length -le 0) {
      $missing += $p
    }
  } catch {
    $missing += $p
  }
}
if ($missing.Count -gt 0) {
  Write-Host "error: missing required model files:" -ForegroundColor Red
  foreach ($m in $missing) { Write-Host "  $m" -ForegroundColor Red }
  throw "re-run bootstrap: powershell -ExecutionPolicy Bypass -File scripts\\bootstrap_chatterbox_multilingual_onnx.ps1 -ModelDir `"$ModelDir`""
}

$hostLine = (& rustc -Vv | Select-String -Pattern '^host:' | Select-Object -First 1)
if (-not $hostLine) {
  throw "failed to detect host triple from: rustc -Vv"
}
$hostTriple = ($hostLine.Line -split '\s+')[1]
$pkgTriple = $hostTriple
if (-not [string]::IsNullOrWhiteSpace($Target)) {
  $pkgTriple = $Target
}

Write-Host "== Build =="
Write-Host "root: $root"
Write-Host "profile: $Profile"
Write-Host "target: $([string]::IsNullOrWhiteSpace($Target) ? '<host>' : $Target)"

$buildDir = "release"
if ($Profile -eq "dev") {
  $buildDir = "debug"
}

if ([string]::IsNullOrWhiteSpace($Target)) {
  if ($Profile -eq "release") {
    & cargo build --release
  } else {
    & cargo build
  }
  $binSrc = Join-Path $root ("target\\" + $buildDir + "\\soundtest.exe")
} else {
  if ($Profile -eq "release") {
    & cargo build --release --target $Target
  } else {
    & cargo build --target $Target
  }
  $binSrc = Join-Path $root ("target\\" + $Target + "\\" + $buildDir + "\\soundtest.exe")
}

if (-not (Test-Path -LiteralPath $binSrc -PathType Leaf)) {
  throw "built binary not found: $binSrc"
}

$pkgDir = Join-Path $OutDir ("soundtest-" + $pkgTriple)
if (Test-Path -LiteralPath $pkgDir) {
  Remove-Item -Recurse -Force -LiteralPath $pkgDir
}
New-Item -ItemType Directory -Force -Path $pkgDir | Out-Null

Write-Host ""
Write-Host "== Package =="
Write-Host "dest: $pkgDir"

Copy-Item -Force -LiteralPath $binSrc -Destination (Join-Path $pkgDir "soundtest.exe")

$modelsRoot = Join-Path $pkgDir "models"
New-Item -ItemType Directory -Force -Path $modelsRoot | Out-Null

Write-Host "copying model dir (this can be large)..."
Copy-Item -Recurse -Force -LiteralPath $ModelDir -Destination $modelsRoot

$readme = @"
soundtest (offline ONNX TTS)

Quick start:
  .\\soundtest.exe doctor
  .\\soundtest.exe speak dragon "Hello" --no-ai --backend onnx

Notes:
- Run from this directory so the bundled .\\models\\chatterbox-multilingual-onnx is auto-detected.
- If you move the binary elsewhere, pass --onnx-model-dir or set it in %USERPROFILE%\\.soundtest\\config.toml.
"@

Set-Content -Encoding UTF8 -LiteralPath (Join-Path $pkgDir "README.txt") -Value $readme

Write-Host ""
Write-Host "Done."
Write-Host "Test:"
Write-Host "  cd `"$pkgDir`""
Write-Host "  .\\soundtest.exe doctor"
Write-Host "  .\\soundtest.exe speak dragon `"Hello`" --no-ai --backend onnx"

if ($Archive) {
  Write-Host ""
  Write-Host "== Archive =="
  $zipPath = Join-Path $OutDir ("soundtest-" + $pkgTriple + ".zip")
  if (Test-Path -LiteralPath $zipPath) {
    Remove-Item -Force -LiteralPath $zipPath
  }
  Compress-Archive -Force -Path $pkgDir -DestinationPath $zipPath
  Write-Host "ok: $zipPath"
}
