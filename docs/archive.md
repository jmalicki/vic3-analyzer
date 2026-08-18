# Local archive

Past saves and **alternative plans** are first-class. Nothing is uploaded.

## AnalysisRecord

Shared serde / schemars type (same crate as option structs). No `PathBuf`.

| Field | Meaning |
| --- | --- |
| `id` | uuid |
| `created_at` | RFC 3339 |
| `label` | optional (“rush munitions”, “law first”) |
| `kind` | `prices` \| `what_if` \| `gaps` \| `plan` |
| `fingerprint` | hash of save bytes |
| `date` | in-game date |
| `country` | tag |
| `filename` | original name, informational |
| `opts` | the shared option struct used |
| `result` | same JSON the command emits |
| `limitations` | solver limitation strings |
| `parent_id` | optional; this record is an alternative to another |
| `blob` | optional save bytes (and token map if binary) |

**I9:** JSON round-trip preserves id, fingerprint, kind, opts, result. Compare of two identical records is an empty diff.

## Storage

| Client | Store |
| --- | --- |
| CLI | `$XDG_DATA_HOME/vic3-analyzer/` (else `~/.local/share/vic3-analyzer/`): one JSON file per record; blobs beside them |
| UI analyses | IndexedDB `vic3-analyzer` (`localStorage` is too small for `.v3`) |
| UI saves | IndexedDB `vic3-analyzer-save`: origins, timelines, steps, and a `current` pointer |
| wasm | serialize/parse only; does not own the store |

CLI default: fingerprint + path, no blob. `--archive-blob` copies bytes. UI default: store the blob on drop so “open this past save” works without the file.

Export/import JSON so a CLI record can be dropped into the UI and vice versa.

## Origins, timelines, steps, current

The browser save database is a campaign tree, not a flat list of last-dropped files.

| Store | Meaning |
| --- | --- |
| `origins` | One dropped `.v3` (bytes/blob, optional tokens, fingerprint, name). Never mutated. |
| `timelines` | Labeled branches on an origin (`Main` after drop). Indexed by `origin_id`. |
| `steps` | Nodes on a timeline: `parent_step_id`, `mutations`, cached prices, optional patched bytes. Indexed by `timeline_id`. |
| `current` | Singleton pointer `{ origin_id, timeline_id, step_id }` for the open campaign. |

Dropping a file creates an origin, a Main timeline, a root step, and sets `current`. Preview mutations stay in wasm; committing a step records the delta and can keep patched plaintext bytes. Checking out another origin/timeline/step restores that save without re-reading the original file. Reloading the tab follows `current`.

Analysis records (`AnalysisRecord`) remain a separate store: named prices / what-if / gaps / plan runs keyed by fingerprint.

## Browse

List by fingerprint / in-game date / country. Several labeled plans may share a fingerprint (alternatives on one snapshot). Different fingerprints are the campaign over time.

Reopen from a stored blob, or re-drop a file whose fingerprint matches.

## Compare (P11)

Pick two records. Same fingerprint = alternative plans; different = progression.

Diff stored results (do not re-run A* unless the user asks to replan):

- **plan:** day cost, aligned action sequence, techs/laws/buildings queued
- **prices:** per-good Δ
- **gaps:** predicates still failing vs cleared

CLI: `archive list` / `show` / `diff <id> <id>` emitting the same compare JSON as the UI.

## Non-goals

Cloud sync, uploading saves, treating the archive as a git repo of `.v3` files.
