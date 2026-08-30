#!/usr/bin/env bash
# Headless MCP smoke for the fat `vic3-analyzer` binary (Wave 4b).
#
# Why this exists: prove the early argv branch reaches stdio MCP without a
# display / without calling Tauri `run()`. A greppable "mcp ready" on stderr is
# the readiness signal (JSON-RPC stays on stdout).
#
# WebView caveat: this binary still *links* Tauri, so WKWebView/WebKitGTK/WebView2
# may map at process start. That is acceptable for v1 and is not what we test
# here — we only assert no window / headless ready.
#
# Isolation: write `auto_detect = false` under a private XDG_DATA_HOME so a
# developer Steam install cannot trigger a multi-minute defs rebuild during smoke.
#
# After ready we SIGTERM the server: rmcp may keep the process alive waiting for
# a full JSON-RPC session, and this smoke only cares about headless startup.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

cargo_target_profile() {
  case "${CARGO_PROFILE:-dev}" in
    dev) echo debug ;;
    *) echo "${CARGO_PROFILE:-dev}" ;;
  esac
}

BIN="${VIC3_ANALYZER_BIN:-}"
if [[ -z "$BIN" ]]; then
  # Prefer an explicit cargo (CI / rustup toolchain) when the environment wraps
  # `cargo` as a rustup shim that does not forward subcommands.
  CARGO_BIN="${CARGO:-cargo}"
  profile_args=()
  if [[ -n "${CARGO_PROFILE:-}" && "${CARGO_PROFILE}" != dev ]]; then
    profile_args=(--profile "$CARGO_PROFILE")
  fi
  "$CARGO_BIN" build -p vic3-analyzer --quiet "${profile_args[@]}"
  BIN="$ROOT/target/$(cargo_target_profile)/vic3-analyzer"
fi

if [[ ! -x "$BIN" ]]; then
  echo "error: binary not executable: $BIN" >&2
  exit 1
fi

# Empty DISPLAY: on Linux CI, creating a window would typically fail. We still
# rely on the argv branch (never calling Tauri run) as the real guarantee.
export DISPLAY="${DISPLAY:-}"
unset WAYLAND_DISPLAY || true

tmpdir="$(mktemp -d)"
pid=""
cleanup() {
  [[ -n "${pid}" ]] && kill "$pid" 2>/dev/null || true
  rm -rf "$tmpdir"
}
trap cleanup EXIT

stderr_log="$tmpdir/stderr.log"
# Keep stdin open on a FIFO so the server blocks in the stdio loop after ready
# instead of exiting immediately on EOF before logging ready.
fifo="$tmpdir/stdin.fifo"
mkfifo "$fifo"
exec 3<>"$fifo"

# Private app-data: never touch the developer’s real config; disable auto-detect
# so smoke does not build defs from a live Victoria 3 install.
export XDG_DATA_HOME="$tmpdir/xdg"
app_data="$XDG_DATA_HOME/vic3-analyzer"
mkdir -p "$app_data"
cat >"$app_data/config.toml" <<'EOF'
auto_detect = false
EOF

"$BIN" mcp <"$fifo" >"$tmpdir/stdout.log" 2>"$stderr_log" &
pid=$!

ready=0
for _ in $(seq 1 80); do
  if grep -q "vic3-analyzer mcp ready" "$stderr_log" 2>/dev/null; then
    ready=1
    break
  fi
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "error: mcp process exited before ready" >&2
    cat "$stderr_log" >&2 || true
    exit 1
  fi
  sleep 0.25
done

if [[ "$ready" -ne 1 ]]; then
  echo "error: timed out waiting for mcp ready on stderr" >&2
  cat "$stderr_log" >&2 || true
  exit 1
fi

kill "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
pid=""
echo "mcp smoke ok (no window; headless ready)"
