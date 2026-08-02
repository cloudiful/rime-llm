#!/usr/bin/env bash
set -euo pipefail

readonly TARGET="${TARGET:?TARGET is required}"
readonly OUTPUT_DIR="package-output"

version="manual-${GITHUB_RUN_ID:-local}"
if [[ "${GITHUB_REF_TYPE:-}" == "tag" ]]; then
  version="${GITHUB_REF_NAME}"
fi

readonly ARCHIVE="rime-llm-${version}-macos-arm64.tar.gz"
rm -rf package "${OUTPUT_DIR}"
mkdir -p package "${OUTPUT_DIR}"
cp "target/${TARGET}/release/rime-llm" package/rime-llm
cp "target/${TARGET}/release/ime-daemon" package/ime-daemon
cp config.example.toml package/config.example.toml
mkdir -p package/data
cp -R data/rime-ice package/data/rime-ice
if [[ -d "build/ime/RimeLLMInputMethod.app" ]]; then
  cp -R "build/ime/RimeLLMInputMethod.app" package/RimeLLMInputMethod.app
fi
test -x package/rime-llm
test -x package/ime-daemon
tar -czf "${OUTPUT_DIR}/${ARCHIVE}" -C package .
(
  cd "${OUTPUT_DIR}"
  shasum -a 256 "${ARCHIVE}"
) > "${OUTPUT_DIR}/${ARCHIVE}.sha256"
