# E2E save fixtures (PR #76 plan)

Materialized as `*.v3` at test start (`prepareE2eSaves`) because `*.v3` is gitignored.

| Save | Source | Expected UI markers |
|------|--------|---------------------|
| `mock_shortage.v3` | `mock_shortage.txt` | `mock_lumber`, Mock Lumber Camp, Mock Tool Workshop; lumber pressure / shortage path |
| `mock_balanced.v3` | `mock_balanced.txt` | `mock_lumber`, Mock Lumber Camp; **no** Mock Tool Workshop |
| `mock_two_countries.v3` | `mock_two_countries.txt` | Player `MOCK`/`Home` + foreign `RIVAL`/`Rivalia` (other market) — scope / alert tests |

Defs: `mock_game/` tree (Tauri `game_dir`) and `mock_game.defs.postcard` (web Definitions file upload).
