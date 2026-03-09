#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

TARGET="${SOUNDTEST_TARGET:-}"
PROFILE="${SOUNDTEST_PROFILE:-release}"
MODEL_DIR="${SOUNDTEST_MODEL_DIR:-$ROOT/models/chatterbox-multilingual-onnx}"
DIST_ROOT="${SOUNDTEST_DIST_ROOT:-$ROOT/dist}"
BOOTSTRAP="${SOUNDTEST_BOOTSTRAP:-1}"
ARCHIVE="${SOUNDTEST_ARCHIVE:-0}"

usage() {
  cat <<'EOF'
Usage:
  bash scripts/build_dist.sh [--target <triple>] [--profile release|dev] [--model-dir <dir>] [--out <dir>] [--no-bootstrap] [--archive]

Environment:
  SOUNDTEST_TARGET        Rust target triple (optional)
  SOUNDTEST_PROFILE       release|dev (default: release)
  SOUNDTEST_MODEL_DIR     Model dir to include (default: ./models/chatterbox-multilingual-onnx)
  SOUNDTEST_DIST_ROOT     Dist output root (default: ./dist)
  SOUNDTEST_BOOTSTRAP     1 to auto-download missing assets (default: 1)
  SOUNDTEST_ARCHIVE       1 to create a .tar.gz (default: 0)
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target)
      TARGET="${2:-}"
      shift 2
      ;;
    --profile)
      PROFILE="${2:-}"
      shift 2
      ;;
    --model-dir)
      MODEL_DIR="${2:-}"
      shift 2
      ;;
    --out)
      DIST_ROOT="${2:-}"
      shift 2
      ;;
    --no-bootstrap)
      BOOTSTRAP="0"
      shift 1
      ;;
    --archive)
      ARCHIVE="1"
      shift 1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown arg: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "$PROFILE" != "release" && "$PROFILE" != "dev" ]]; then
  echo "error: --profile must be release or dev (got: $PROFILE)" >&2
  exit 2
fi

if [[ ! -d "$MODEL_DIR" ]]; then
  if [[ "$BOOTSTRAP" == "1" ]]; then
    echo "note: model dir missing: $MODEL_DIR" >&2
    echo "      bootstrapping assets..." >&2
    (cd "$ROOT" && SOUNDTEST_MODEL_DIR="$MODEL_DIR" bash scripts/bootstrap_chatterbox_multilingual_onnx.sh)
  else
    echo "note: model dir missing: $MODEL_DIR" >&2
    echo "      Run: bash scripts/bootstrap_chatterbox_multilingual_onnx.sh" >&2
    exit 1
  fi
fi

required_files=(
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

missing=0
for f in "${required_files[@]}"; do
  if [[ ! -s "$MODEL_DIR/$f" ]]; then
    echo "error: missing required model file: $MODEL_DIR/$f" >&2
    missing=1
  fi
done
if [[ $missing -ne 0 ]]; then
  echo "hint: re-run bootstrap:" >&2
  echo "  SOUNDTEST_MODEL_DIR=\"$MODEL_DIR\" bash scripts/bootstrap_chatterbox_multilingual_onnx.sh" >&2
  exit 1
fi

HOST_TRIPLE="$(rustc -Vv | awk '/^host:/{print $2}')"
PKG_TRIPLE="${TARGET:-$HOST_TRIPLE}"

echo "== Build =="
echo "root: $ROOT"
echo "profile: $PROFILE"
echo "target: ${TARGET:-<host>}"

BUILD_DIR="release"
if [[ "$PROFILE" == "dev" ]]; then
  BUILD_DIR="debug"
fi

if [[ -n "$TARGET" ]]; then
  if [[ "$PROFILE" == "release" ]]; then
    cargo build --release --target "$TARGET"
  else
    cargo build --target "$TARGET"
  fi
  BIN_SRC="$ROOT/target/$TARGET/$BUILD_DIR/soundtest"
else
  if [[ "$PROFILE" == "release" ]]; then
    cargo build --release
  else
    cargo build
  fi
  BIN_SRC="$ROOT/target/$BUILD_DIR/soundtest"
fi

if [[ ! -x "$BIN_SRC" ]]; then
  echo "error: built binary not found: $BIN_SRC" >&2
  exit 1
fi

PKG_DIR="$DIST_ROOT/soundtest-$PKG_TRIPLE"
rm -rf "$PKG_DIR"
mkdir -p "$PKG_DIR"

echo
echo "== Package =="
echo "dest: $PKG_DIR"

cp -f "$BIN_SRC" "$PKG_DIR/"

mkdir -p "$PKG_DIR/models"
echo "copying model dir (this can be large)..."
cp -a "$MODEL_DIR" "$PKG_DIR/models/"

cat >"$PKG_DIR/README.txt" <<'EOF'
soundtest (offline ONNX TTS)

Quick start:
  ./soundtest doctor
  ./soundtest speak dragon "Hello" --no-ai --backend onnx

Notes:
- Run from this directory so the bundled ./models/chatterbox-multilingual-onnx is auto-detected.
- If you move the binary elsewhere, pass --onnx-model-dir or set it in ~/.soundtest/config.toml.
EOF

echo
echo "Done."
echo "Test:"
echo "  (cd \"$PKG_DIR\" && ./soundtest doctor)"
echo "  (cd \"$PKG_DIR\" && ./soundtest speak dragon \"Hello\" --no-ai --backend onnx)"

if [[ "$ARCHIVE" == "1" ]]; then
  echo
  echo "== Archive =="
  ARCHIVE_PATH="$DIST_ROOT/soundtest-$PKG_TRIPLE.tar.gz"
  rm -f "$ARCHIVE_PATH"
  tar -czf "$ARCHIVE_PATH" -C "$DIST_ROOT" "soundtest-$PKG_TRIPLE"
  echo "ok: $ARCHIVE_PATH"
fi
