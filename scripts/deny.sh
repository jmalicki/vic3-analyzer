#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo deny --version
cargo deny check advisories sources bans licenses
