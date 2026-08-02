#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

CONFIGURATION="${CONFIGURATION:-Release}"
PROJECT="macos/RimeLLMInputMethod/RimeLLMInputMethod.xcodeproj"
TARGET="RimeLLMInputMethod"
OUTPUT_DIR="${OUTPUT_DIR:-$PWD/build/ime}"
ARCHS="${ARCHS:-arm64}"
DICTIONARY_ROOT="${RIME_LLM_DICTIONARY_ROOT:-data/rime-ice}"

if [[ -n "${IME_DAEMON_BIN:-}" ]]; then
  DAEMON_BIN="$IME_DAEMON_BIN"
else
  DAEMON_BIN="target/release/ime-daemon"
  cargo build --release -p ime-daemon
fi

xcodebuild -project "$PROJECT" -target "$TARGET" -configuration "$CONFIGURATION" \
  ARCHS="$ARCHS" ONLY_ACTIVE_ARCH=NO \
  CONFIGURATION_BUILD_DIR="$OUTPUT_DIR" SYMROOT="$OUTPUT_DIR" \
  OBJROOT="$OUTPUT_DIR/obj" MODULE_CACHE_DIR="$OUTPUT_DIR/ModuleCache" \
  build
rm -rf macos/RimeLLMInputMethod/DerivedData macos/RimeLLMInputMethod/build

APP_DIR="$OUTPUT_DIR/RimeLLMInputMethod.app"
mkdir -p "$APP_DIR/Contents/Resources"
cp "$DAEMON_BIN" "$APP_DIR/Contents/Resources/ime-daemon"
test -x "$APP_DIR/Contents/Resources/ime-daemon"
if [[ -d "$DICTIONARY_ROOT" ]]; then
  rm -rf "$APP_DIR/Contents/Resources/data"
  mkdir -p "$APP_DIR/Contents/Resources/data"
  cp -R "$DICTIONARY_ROOT" "$APP_DIR/Contents/Resources/data/rime-ice"
fi
chmod +x "$APP_DIR/Contents/Resources/ime-daemon"
codesign --force --deep --sign - "$APP_DIR"
echo "Built $APP_DIR"
