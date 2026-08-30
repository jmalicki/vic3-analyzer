# Supply chain security

This repo gates Rust and JavaScript dependencies in CI and via Dependabot.

## Rust (`cargo-deny`)

[`deny.toml`](../deny.toml) configures [cargo-deny](https://github.com/EmbarkStudios/cargo-deny):

| Check | Purpose |
|-------|---------|
| **advisories** | RustSec CVEs; workspace-scoped unmaintained warnings |
| **licenses** | Allow-list aligned with AGPL-3.0 distribution |
| **bans** | Deny wildcard version requirements in manifests |
| **sources** | Crates.io + allowlisted git URLs only |

CI runs `./scripts/deny.sh` (cargo-deny 0.18 via taiki-e/install-action) and a
production JS dependency audit in the dedicated `supply-chain` job, separate
from compile, test, and build jobs.

Local Rust checks:

```bash
cargo install cargo-deny --locked
./scripts/deny.sh
```

Local JS audit (npm, until pnpm workspace lands):

```bash
cd web && npm ci && npm audit --omit=dev --audit-level=high
```

### Changing policy

- **New git dependency:** add URL under `[sources.allow-git]` in `deny.toml` (requires PR review).
- **License exception:** add `[[licenses.exceptions]]` or `[[licenses.clarify]]` with a reason (last resort).
- **Advisory ignore:** add to `[advisories].ignore` with issue link and removal plan.

We do not use [cargo-vet](https://github.com/mozilla/cargo-vet) today; the bootstrap cost for ~500+ transitive crates is high for a small team.

## JavaScript

- **Dependabot** (`.github/dependabot.yml`): weekly npm + cargo updates with 3-day cooldown.
- **pnpm** (after workspace migration): `minimumReleaseAge` and `trustPolicy` in `pnpm-workspace.yaml`.
- **CI audit:** production dependency audit in the `supply-chain` job (`pnpm audit --prod` after pnpm migration; `npm audit --omit=dev` before).

## Related

- [CONTRIBUTING.md](../CONTRIBUTING.md) — dev setup
- [RustSec advisory database](https://rustsec.org/)
