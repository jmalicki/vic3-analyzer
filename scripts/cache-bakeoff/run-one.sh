#!/usr/bin/env bash
# Measure one compressor × setting: write size/timing JSON only.
# Does not upload to GitHub Actions cache — archives are deleted after measuring.
set -euo pipefail

NAME="${1:?name}"
CORPUS_ZST="${2:?corpus.tar.zst}"
OUT_JSON="${3:?out.json}"
COMPILE_SECS="${COMPILE_SECS:-0}"
FETCH_SECS="${FETCH_SECS:-0}"

WORKDIR="${RUNNER_TEMP:-/tmp}/bakeoff-$NAME"
mkdir -p "$WORKDIR"
ARCHIVE="$WORKDIR/out.bin"

# Materialize uncompressed tar once (shared input for fair compress timing).
TAR="$WORKDIR/corpus.tar"
echo "Decompressing transport corpus..."
zstd -d --long=31 -T0 -o "$TAR" "$CORPUS_ZST"
TAR_BYTES=$(wc -c <"$TAR" | tr -d ' ')

compress() {
  case "$NAME" in
    zstd-3-long30-actions) zstd -3 --long=30 -T0 -c "$TAR" >"$ARCHIVE" ;;
    zstd-3-long31)         zstd -3 --long=31 -T0 -c "$TAR" >"$ARCHIVE" ;;
    zstd-10-long31)        zstd -10 --long=31 -T0 -c "$TAR" >"$ARCHIVE" ;;
    zstd-19-long31)        zstd -19 --long=31 -T0 -c "$TAR" >"$ARCHIVE" ;;
    zstd-22-ultra-long31)  zstd -22 --ultra --long=31 -T0 -c "$TAR" >"$ARCHIVE" ;;
    xz-6)                  xz -6 -T0 -c "$TAR" >"$ARCHIVE" ;;
    xz-9)                  xz -9 -T0 -c "$TAR" >"$ARCHIVE" ;;
    xz-9e)                 xz -9e -T0 -c "$TAR" >"$ARCHIVE" ;;
    brotli-5)              brotli -q 5 -c "$TAR" >"$ARCHIVE" ;;
    brotli-11)             brotli -q 11 -c "$TAR" >"$ARCHIVE" ;;
    lz4-9)                 lz4 -9 -c "$TAR" >"$ARCHIVE" ;;
    pigz-9)                pigz -9 -c "$TAR" >"$ARCHIVE" ;;
    *) echo "unknown codec $NAME" >&2; exit 1 ;;
  esac
}

decompress() {
  case "$NAME" in
    zstd-3-long30-actions) zstd -d --long=30 -T0 -c <"$ARCHIVE" >/dev/null ;;
    zstd-*-long31|zstd-22-ultra-long31) zstd -d --long=31 -T0 -c <"$ARCHIVE" >/dev/null ;;
    xz-*)                  xz -d -T0 -c <"$ARCHIVE" >/dev/null ;;
    brotli-*)              brotli -d -c <"$ARCHIVE" >/dev/null ;;
    lz4-9)                 lz4 -d -c <"$ARCHIVE" >/dev/null ;;
    pigz-9)                pigz -d -c <"$ARCHIVE" >/dev/null ;;
  esac
}

echo "Compressing with $NAME..."
T0=$(date +%s)
compress
T1=$(date +%s)
echo "Decompressing $NAME..."
decompress
T2=$(date +%s)

SIZE=$(wc -c <"$ARCHIVE" | tr -d ' ')
CSECS=$((T1 - T0))
DSECS=$((T2 - T1))
FETCH_PLUS=$((FETCH_SECS + COMPILE_SECS))

python3 - "$OUT_JSON" <<PY
import json, os, sys
out = sys.argv[1]
size = int("$SIZE")
csecs = int("$CSECS")
dsecs = int("$DSECS")
compile_secs = int("$COMPILE_SECS")
fetch_secs = int("$FETCH_SECS")
fetch_plus = int("$FETCH_PLUS")
tar_bytes = int("$TAR_BYTES")

def ratio(n, d):
    return None if d <= 0 else round(n / d, 4)

payload = {
    "name": "$NAME",
    "size_bytes": size,
    "tar_bytes": tar_bytes,
    "compress_s": csecs,
    "decompress_s": dsecs,
    "compile_s": compile_secs,
    "fetch_s": fetch_secs,
    "fetch_plus_compile_s": fetch_plus,
    "compress_vs_compile": ratio(csecs, compile_secs),
    "decompress_vs_compile": ratio(dsecs, compile_secs),
    "compress_vs_fetch": ratio(csecs, fetch_secs),
    "decompress_vs_fetch": ratio(dsecs, fetch_secs),
    "compress_vs_fetch_plus_compile": ratio(csecs, fetch_plus),
    "decompress_vs_fetch_plus_compile": ratio(dsecs, fetch_plus),
}
with open(out, "w") as f:
    json.dump(payload, f, indent=2)
    f.write("\n")
print(json.dumps(payload, indent=2))
PY

# Drop large temps
rm -f "$TAR" "$ARCHIVE"
