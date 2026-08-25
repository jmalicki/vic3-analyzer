# E2E save fixtures

Materialized as `*.v3` at test start (`prepareE2eSaves`) because `*.v3` is gitignored.

| Save | Source | Expected UI markers |
|------|--------|---------------------|
| `mock_shortage.v3` | `mock_shortage.txt` | `mock_lumber`, Mock Lumber Camp, Mock Tool Workshop; lumber pressure / shortage path |

Defs: `../mock_game/` tree (Tauri `game_dir`) and [`../mock_game.defs.postcard`](../mock_game.defs.postcard)
(web Definitions upload / Tauri `#cfg-defs`). Regenerate with
`cargo run -q -p vic3-defs --bin emit_fixture_blob -- tests/fixtures/mock_game.defs.postcard tests/fixtures/mock_game`
when `GameDefs` postcard layout / `BLOB_VERSION` changes.
