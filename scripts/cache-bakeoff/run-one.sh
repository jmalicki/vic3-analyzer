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

# pv -i: throttle progress in CI logs (default 10s; avoid per-second spam).
PROGRESS_INTERVAL="${BAKEOFF_PROGRESS_INTERVAL:-10}"

pv_in() {
  local label="$1"
  local size="$2"
  shift 2
  pv -f -i "$PROGRESS_INTERVAL" -F "$label" -s "$size" "$@"
}

# Materialize uncompressed tar once (shared input for fair compress timing).
TAR="$WORKDIR/corpus.tar"
CORPUS_BYTES=$(wc -c <"$CORPUS_ZST" | tr -d ' ')
echo "Decompressing transport corpus..."
pv_in transport "$CORPUS_BYTES" "$CORPUS_ZST" | zstd -d --long=31 -T0 -c >"$TAR"
TAR_BYTES=$(wc -c <"$TAR" | tr -d ' ')

compress() {
  case "$NAME" in
    zstd-3-long30-actions) pv_in "$NAME" "$TAR_BYTES" "$TAR" | zstd -3 --long=30 -T0 -c >"$ARCHIVE" ;;
    zstd-3-long31)         pv_in "$NAME" "$TAR_BYTES" "$TAR" | zstd -3 --long=31 -T0 -c >"$ARCHIVE" ;;
    zstd-10-long31)        pv_in "$NAME" "$TAR_BYTES" "$TAR" | zstd -10 --long=31 -T0 -c >"$ARCHIVE" ;;
    zstd-11-long31)        pv_in "$NAME" "$TAR_BYTES" "$TAR" | zstd -11 --long=31 -T0 -c >"$ARCHIVE" ;;
    zstd-12-long31)        pv_in "$NAME" "$TAR_BYTES" "$TAR" | zstd -12 --long=31 -T0 -c >"$ARCHIVE" ;;
    xz-6)                  pv_in "$NAME" "$TAR_BYTES" "$TAR" | xz -6 -T0 -c >"$ARCHIVE" ;;
    xz-9)                  pv_in "$NAME" "$TAR_BYTES" "$TAR" | xz -9 -T0 -c >"$ARCHIVE" ;;
    xz-9e)                 pv_in "$NAME" "$TAR_BYTES" "$TAR" | xz -9e -T0 -c >"$ARCHIVE" ;;
    brotli-5)              pv_in "$NAME" "$TAR_BYTES" "$TAR" | brotli -q 5 -c >"$ARCHIVE" ;;
    brotli-11)             pv_in "$NAME" "$TAR_BYTES" "$TAR" | brotli -q 11 -c >"$ARCHIVE" ;;
    lz4-9)                 pv_in "$NAME" "$TAR_BYTES" "$TAR" | lz4 -9 -c >"$ARCHIVE" ;;
    pigz-9)                pv_in "$NAME" "$TAR_BYTES" "$TAR" | pigz -9 -c >"$ARCHIVE" ;;
    *) echo "unknown codec $NAME" >&2; exit 1 ;;
  esac
}

decompress() {
  case "$NAME" in
    zstd-3-long30-actions) zstd -d --long=30 -T0 -c <"$ARCHIVE" >/dev/null ;;
    zstd-*-long31) zstd -d --long=31 -T0 -c <"$ARCHIVE" >/dev/null ;;
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
