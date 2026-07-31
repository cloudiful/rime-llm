#!/usr/bin/env bash
set -euo pipefail

readonly SQUIRREL_VERSION="1.1.2"
readonly SQUIRREL_PKG_SHA256="614746013212937623d5bbab9901e9c43d1ec937aa32307d6b6092a05e308287"
readonly LIBRIME_VERSION="1.16.0"
readonly PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly OUTPUT_DIR="${1:?usage: build-native-plugin.sh <output-directory>}"
readonly RUN_ROOT="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/rime-llm-native"

cleanup() {
  rm -rf "${RUN_ROOT}"
}
trap cleanup EXIT

rm -rf "${RUN_ROOT}"
mkdir -p "${RUN_ROOT}" "${OUTPUT_DIR}"

readonly SQUIRREL_PKG="${RUN_ROOT}/Squirrel-${SQUIRREL_VERSION}.pkg"
readonly SQUIRREL_EXPANDED="${RUN_ROOT}/squirrel-expanded"
readonly LIBRIME_SOURCE="${RUN_ROOT}/librime-${LIBRIME_VERSION}"
readonly BUILD_DIR="${RUN_ROOT}/build"

curl --fail --location --retry 3 --retry-all-errors \
  "https://github.com/rime/squirrel/releases/download/${SQUIRREL_VERSION}/Squirrel-${SQUIRREL_VERSION}.pkg" \
  --output "${SQUIRREL_PKG}"
printf '%s  %s\n' "${SQUIRREL_PKG_SHA256}" "${SQUIRREL_PKG}" \
  | shasum --algorithm 256 --check --strict

pkgutil --expand-full "${SQUIRREL_PKG}" "${SQUIRREL_EXPANDED}"
readonly SQUIRREL_APP="$(find "${SQUIRREL_EXPANDED}" -type d -name Squirrel.app -print -quit)"
readonly RIME_LIBRARY="${SQUIRREL_APP}/Contents/Frameworks/librime.1.dylib"
test -n "${SQUIRREL_APP}"
test -f "${RIME_LIBRARY}"
otool -L "${RIME_LIBRARY}" | grep -F 'current version 1.16.0'

git clone --depth 1 --branch "${LIBRIME_VERSION}" \
  https://github.com/rime/librime.git "${LIBRIME_SOURCE}"

if ! brew list --versions boost >/dev/null 2>&1; then
  brew install boost
fi
readonly BOOST_INCLUDE_DIR="$(brew --prefix boost)/include"

cmake -S "${PROJECT_ROOT}/native" -B "${BUILD_DIR}" \
  -DRIME_SOURCE_DIR="${LIBRIME_SOURCE}" \
  -DRIME_LIBRARY="${RIME_LIBRARY}" \
  -DBOOST_INCLUDE_DIR="${BOOST_INCLUDE_DIR}" \
  -DCMAKE_OSX_ARCHITECTURES=arm64 \
  -DCMAKE_CXX_COMPILER_LAUNCHER="${CMAKE_CXX_COMPILER_LAUNCHER:-sccache}"
cmake --build "${BUILD_DIR}" --parallel
ctest --test-dir "${BUILD_DIR}" --output-on-failure

readonly DYLIB="${BUILD_DIR}/rime-plugins/librime-llm-predict.dylib"
test -f "${DYLIB}"
file "${DYLIB}" | grep -F 'arm64'
otool -L "${DYLIB}" | grep -F '@rpath/librime.1.dylib'
install -m 0755 "${DYLIB}" "${OUTPUT_DIR}/librime-llm-predict.dylib"
