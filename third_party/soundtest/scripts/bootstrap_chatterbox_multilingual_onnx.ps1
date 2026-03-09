param(
  [string]$ModelDir = ""
)

$ErrorActionPreference = "Stop"

$root = Resolve-Path (Join-Path $PSScriptRoot "..")
if ([string]::IsNullOrWhiteSpace($ModelDir)) {
  $ModelDir = Join-Path $root "models\\chatterbox-multilingual-onnx"
}

$hfBase = $env:SOUNDTEST_HF_BASE
if ([string]::IsNullOrWhiteSpace($hfBase)) {
  $hfBase = "https://hf-mirror.com/onnx-community/chatterbox-multilingual-ONNX/resolve/main"
}

$files = @(
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

function Download-File([string]$relPath) {
  $url = "$hfBase/$relPath"
  $dst = Join-Path $ModelDir $relPath
  $dir = Split-Path -Parent $dst
  New-Item -ItemType Directory -Force -Path $dir | Out-Null

  if (Test-Path -LiteralPath $dst -PathType Leaf) {
    $len = (Get-Item -LiteralPath $dst).Length
    if ($len -gt 0) {
      Write-Host "ok: $relPath"
      return
    }
  }

  Write-Host "downloading: $relPath"
  Invoke-WebRequest -Uri $url -OutFile $dst
  Write-Host "ok: $relPath"
}

Write-Host "== Model =="
Write-Host "dest: $ModelDir"
foreach ($f in $files) {
  Download-File $f
}

Write-Host ""
Write-Host "== ONNX Runtime =="

$ortTag = $env:SOUNDTEST_ORT_TAG
if ([string]::IsNullOrWhiteSpace($ortTag)) {
  try {
    $ortTag = (Invoke-RestMethod -Uri "https://api.github.com/repos/microsoft/onnxruntime/releases/latest").tag_name
  } catch {
    $ortTag = "v1.24.1"
  }
}
$ortVer = $ortTag.TrimStart("v")

$arch = $env:PROCESSOR_ARCHITECTURE
if ($arch -eq "ARM64") {
  $asset = "onnxruntime-win-arm64-$ortVer.zip"
} else {
  $asset = "onnxruntime-win-x64-$ortVer.zip"
}

$ortUrl = "https://github.com/microsoft/onnxruntime/releases/download/$ortTag/$asset"
$tmp = Join-Path $env:TEMP ("soundtest-ort-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

$zipPath = Join-Path $tmp $asset
try {
  Write-Host "downloading: $asset"
  Invoke-WebRequest -Uri $ortUrl -OutFile $zipPath
  Expand-Archive -Force -Path $zipPath -DestinationPath $tmp

  $dll = Get-ChildItem -Path $tmp -Recurse -Filter "onnxruntime.dll" | Select-Object -First 1
  if (-not $dll) {
    throw "onnxruntime.dll not found after extracting $asset"
  }
} catch {
  Write-Host "note: GitHub download failed, trying pip wheel instead..."

  $py = Get-Command python -ErrorAction SilentlyContinue
  if (-not $py) { $py = Get-Command python3 -ErrorAction SilentlyContinue }
  if (-not $py) {
    throw "python/python3 not found. Put onnxruntime.dll next to the model dir, or pass --onnx-runtime."
  }

  & $py.Source -m pip download --only-binary :all: --no-deps -q -d $tmp "onnxruntime==$ortVer" 2>$null
  if ($LASTEXITCODE -ne 0) {
    & $py.Source -m pip download --only-binary :all: --no-deps -q -d $tmp "onnxruntime" 2>$null
  }

  $wheel = Get-ChildItem -Path $tmp -Filter "onnxruntime-*.whl" | Select-Object -First 1
  if (-not $wheel) {
    throw "failed to download onnxruntime wheel"
  }

  $wheelExtract = Join-Path $tmp "wheel"
  New-Item -ItemType Directory -Force -Path $wheelExtract | Out-Null
  Expand-Archive -Force -Path $wheel.FullName -DestinationPath $wheelExtract

  $dll = Get-ChildItem -Path $wheelExtract -Recurse -Filter "onnxruntime.dll" | Select-Object -First 1
  if (-not $dll) {
    throw "onnxruntime.dll not found after extracting wheel $($wheel.Name)"
  }
}

New-Item -ItemType Directory -Force -Path $ModelDir | Out-Null

$dllDir = Split-Path -Parent $dll.FullName
$allDlls = Get-ChildItem -Path $dllDir -Filter "onnxruntime*.dll" | Sort-Object Name
if (-not $allDlls -or $allDlls.Count -lt 1) {
  $allDlls = @($dll)
}

foreach ($d in $allDlls) {
  Copy-Item -Force -LiteralPath $d.FullName -Destination (Join-Path $ModelDir $d.Name)
  Write-Host "ok: $($d.Name)"
}

Remove-Item -Recurse -Force -LiteralPath $tmp

Write-Host ""
Write-Host "Done."
Write-Host ""
Write-Host "Test (from repo root):"
Write-Host "  cargo run -- doctor"
Write-Host "  cargo run -- speak dragon \"Hello\" --no-ai"
Write-Host ""
