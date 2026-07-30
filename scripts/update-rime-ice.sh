#!/usr/bin/env bash
set -euo pipefail

readonly ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
readonly DATA_DIR="$ROOT_DIR/data/rime-ice"
readonly SOURCE_RECORD="$DATA_DIR/SOURCE.md"
readonly UPSTREAM_BASE="https://raw.githubusercontent.com/iDvel/rime-ice"

if [[ $# -ne 1 || ! "$1" =~ ^[0-9a-f]{40}$ ]]; then
    printf 'usage: %s <40-character rime-ice commit>\n' "$0" >&2
    exit 64
fi

readonly COMMIT="$1"
readonly SNAPSHOT_DATE="$(date +%F)"
readonly TEMP_DIR="$(mktemp -d /tmp/rime-ice-update.XXXXXX)"
trap 'rm -rf "$TEMP_DIR"' EXIT

hash_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    else
        shasum -a 256 "$1" | awk '{ print $1 }'
    fi
}

for relative_path in \
    LICENSE \
    rime_ice.dict.yaml \
    cn_dicts/8105.dict.yaml \
    cn_dicts/base.dict.yaml \
    cn_dicts/ext.dict.yaml \
    cn_dicts/tencent.dict.yaml \
    cn_dicts/others.dict.yaml
do
    target="$TEMP_DIR/$relative_path"
    mkdir -p "$(dirname "$target")"
    curl --fail --location --retry 3 --silent --show-error \
        "$UPSTREAM_BASE/$COMMIT/$relative_path" \
        --output "$target"
done

cp "$SOURCE_RECORD" "$TEMP_DIR/SOURCE.md"
perl -0pi -e \
    's!^- Snapshot commit:.*$!- Snapshot commit: \x60'"$COMMIT"'\x60!m;
     s!^- Snapshot date:.*$!- Snapshot date: \x60'"$SNAPSHOT_DATE"'\x60!m' \
    "$TEMP_DIR/SOURCE.md"

for relative_path in \
    rime_ice.dict.yaml \
    cn_dicts/8105.dict.yaml \
    cn_dicts/base.dict.yaml \
    cn_dicts/ext.dict.yaml \
    cn_dicts/tencent.dict.yaml \
    cn_dicts/others.dict.yaml \
    LICENSE
do
    digest="$(hash_file "$TEMP_DIR/$relative_path")"
    perl -0pi -e \
        's!^[0-9a-f]{64}  '"$relative_path"'$!'"$digest"'  '"$relative_path"'!m' \
        "$TEMP_DIR/SOURCE.md"
done

for relative_path in \
    LICENSE \
    rime_ice.dict.yaml \
    cn_dicts/8105.dict.yaml \
    cn_dicts/base.dict.yaml \
    cn_dicts/ext.dict.yaml \
    cn_dicts/tencent.dict.yaml \
    cn_dicts/others.dict.yaml
do
    mkdir -p "$DATA_DIR/$(dirname "$relative_path")"
    cp "$TEMP_DIR/$relative_path" "$DATA_DIR/$relative_path"
done
cp "$TEMP_DIR/SOURCE.md" "$SOURCE_RECORD"

printf 'Updated rime-ice dictionary data to %s (%s).\n' "$COMMIT" "$SNAPSHOT_DATE"
