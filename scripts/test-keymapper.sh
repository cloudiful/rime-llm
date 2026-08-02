#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../macos/RimeLLMInputMethod"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
xcrun swiftc -target arm64-apple-macosx13.0 \
  -framework AppKit -framework CoreGraphics \
  RimeLLMInputMethod/KeyMapper.swift RimeLLMInputMethod/DaemonClient.swift Tests/main.swift \
  -o "$TMP/keymapper-tests"
"$TMP/keymapper-tests"
