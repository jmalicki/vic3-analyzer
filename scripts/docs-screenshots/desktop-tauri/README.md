# Real macOS Tauri screenshots (optional)

Not wired in this PR. Ubuntu CI uses [`capture-desktop-mock.mjs`](../capture-desktop-mock.mjs) instead.

When implementing:

1. Run the companion with a disposable `XDG_DATA_HOME` and fixture saves/defs (same idea as MCP smoke).
2. Drive it with Tauri WebDriver on macOS ([docs](https://v2.tauri.app/develop/tests/webdriver/)): `@wdio/tauri-service` + debug-only `tauri-plugin-wdio-webdriver` (embedded provider).
3. Write the same locked filenames as the mock path (`desktop-*.png` in `docs/assets/`).

Keep WebDriver plugins behind debug/docs features so release builds and headless Linux `cargo test` stay clean.
