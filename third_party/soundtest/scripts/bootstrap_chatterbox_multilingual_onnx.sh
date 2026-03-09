#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODEL_DIR="${SOUNDTEST_MODEL_DIR:-$ROOT/models/chatterbox-multilingual-onnx}"
HF_BASE="${SOUNDTEST_HF_BASE:-https://hf-mirror.com/onnx-community/chatterbox-multilingual-ONNX/resolve/main}"

FILES=(
  "tokenizer.json"
  "default_voice.wav"
  "onnx/speech_encoder.onnx"
  "onnx/speech_encoder.onnx_data"
  "onnx/embed_tokens.onnx"
  "onnx/embed_tokens.onnx_data"
  "onnx/conditional_decoder.onnx"
  "onnx/conditional_decoder.onnx_data"
  "onnx/language_model_q4.onnx"
  "onnx/language_model_q4.onnx_data"
)

download() {
  local rel="$1"
  local url="${HF_BASE}/${rel}"
  local dst="${MODEL_DIR}/${rel}"
  mkdir -p "$(dirname "$dst")"
  if [[ -s "$dst" ]]; then
    echo "ok: ${rel}"
    return 0
  fi
  echo "downloading: ${rel}"
  curl -fL -C - --retry 5 --retry-delay 2 -o "$dst" "$url"
  echo "ok: ${rel}"
}

echo "== Model =="
echo "dest: ${MODEL_DIR}"
for f in "${FILES[@]}"; do
  download "$f"
done

OS="$(uname -s)"
ARCH="$(uname -m)"

echo
echo "== ONNX Runtime =="

ORT_TAG="${SOUNDTEST_ORT_TAG:-}"
if [[ -z "${ORT_TAG}" ]]; then
  ORT_TAG="$(
    curl -fsSL https://api.github.com/repos/microsoft/onnxruntime/releases/latest 2>/dev/null \
      | python3 -c "import json,sys; print(json.load(sys.stdin)['tag_name'])" 2>/dev/null \
      || true
  )"
  ORT_TAG="${ORT_TAG:-v1.24.1}"
fi
ORT_VER="${ORT_TAG#v}"

download_ort_via_github() {
  local asset="$1"
  local url="https://github.com/microsoft/onnxruntime/releases/download/${ORT_TAG}/${asset}"
  echo "downloading: ${asset}"
  curl -fL --retry 5 --retry-delay 2 -o "$TMP/ort.tgz" "$url"
  tar -xzf "$TMP/ort.tgz" -C "$TMP"
  DYLIB="$(find "$TMP" -name libonnxruntime.dylib -print -quit || true)"
  if [[ -z "${DYLIB}" ]]; then
    DYLIB="$(find "$TMP" -name 'libonnxruntime*.dylib' -print -quit || true)"
  fi
  if [[ -z "${DYLIB}" ]]; then
    echo "error: libonnxruntime*.dylib not found in ${asset}"
    return 1
  fi
  cp -f "$DYLIB" "$MODEL_DIR/libonnxruntime.dylib"
  echo "ok: libonnxruntime.dylib"

  # Copy any additional onnxruntime dylibs (if present).
  while IFS= read -r lib; do
    local base
    base="$(basename "$lib")"
    if [[ "$base" == "libonnxruntime.dylib" ]]; then
      continue
    fi
    cp -f "$lib" "$MODEL_DIR/$base"
    echo "ok: $base"
  done < <(find "$TMP" -name 'libonnxruntime*.dylib' -print || true)
  return 0
}

download_ort_via_pip() {
  if ! command -v python3 >/dev/null 2>&1; then
    return 1
  fi
  if ! python3 -m pip --version >/dev/null 2>&1; then
    return 1
  fi

  echo "downloading: onnxruntime (pip wheel)"
  python3 -m pip download --only-binary :all: --no-deps -q -d "$TMP" "onnxruntime==${ORT_VER}" \
    || python3 -m pip download --only-binary :all: --no-deps -q -d "$TMP" "onnxruntime"

  local wheel
  wheel="$(ls -1 "$TMP"/onnxruntime-*.whl 2>/dev/null | head -n 1 || true)"
  if [[ -z "${wheel}" ]]; then
    echo "error: failed to download onnxruntime wheel"
    return 1
  fi

  python3 -m zipfile -e "$wheel" "$TMP/wheel"
  DYLIB="$(find "$TMP/wheel" -name 'libonnxruntime.dylib' -print -quit || true)"
  if [[ -z "${DYLIB}" ]]; then
    DYLIB="$(find "$TMP/wheel" -name 'libonnxruntime*.dylib' -print -quit || true)"
  fi
  if [[ -z "${DYLIB}" ]]; then
    echo "error: libonnxruntime*.dylib not found in wheel"
    return 1
  fi
  cp -f "$DYLIB" "$MODEL_DIR/libonnxruntime.dylib"
  echo "ok: libonnxruntime.dylib"

  # Copy any additional onnxruntime dylibs (if present).
  while IFS= read -r lib; do
    local base
    base="$(basename "$lib")"
    if [[ "$base" == "libonnxruntime.dylib" ]]; then
      continue
    fi
    cp -f "$lib" "$MODEL_DIR/$base"
    echo "ok: $base"
  done < <(find "$TMP/wheel" -name 'libonnxruntime*.dylib' -print || true)
  return 0
}

case "${OS}-${ARCH}" in
  Darwin-arm64)
    TMP="$(mktemp -d)"
    trap 'rm -rf "$TMP"' EXIT
    ORT_ASSET="onnxruntime-osx-arm64-${ORT_VER}.tgz"
    download_ort_via_github "$ORT_ASSET" || download_ort_via_pip || {
      echo "error: failed to download ONNX Runtime. Put libonnxruntime.dylib next to the model dir, or pass --onnx-runtime."
      exit 1
    }
    ;;
  Darwin-x86_64)
    TMP="$(mktemp -d)"
    trap 'rm -rf "$TMP"' EXIT
    ORT_ASSET="onnxruntime-osx-x86_64-${ORT_VER}.tgz"
    download_ort_via_github "$ORT_ASSET" || download_ort_via_pip || {
      echo "error: failed to download ONNX Runtime. Put libonnxruntime.dylib next to the model dir, or pass --onnx-runtime."
      exit 1
    }
    ;;
  *)
    echo "note: automatic ONNX Runtime download is implemented for macOS only in this script."
    echo "      Put onnxruntime.dll / libonnxruntime.dylib next to the model dir, or pass --onnx-runtime."
    ;;
esac

echo
echo "Done."
echo
echo "Test (from repo root):"
echo "  cargo run -- doctor"
echo "  cargo run -- speak dragon \"Hello\" --no-ai"
echo
