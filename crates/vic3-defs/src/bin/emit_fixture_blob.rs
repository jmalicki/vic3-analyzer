//! Emit a postcard defs blob from an in-repo fixture tree for local development
//! and tests. It stays out of `web/public/` so production builds ship no demo
//! definitions; a deployed site has none until the user builds their own.
//!
//! Usage:
//! ```text
//! cargo run -p vic3-defs --bin emit_fixture_blob -- web/fixtures/defs.postcard
//! cargo run -p vic3-defs --bin emit_fixture_blob -- tests/fixtures/mock_game.defs.postcard path/to/mock_game
//! ```

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn default_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(out) = args.next() else {
        eprintln!("usage: emit_fixture_blob <output.postcard> [fixture_root]");
        return ExitCode::FAILURE;
    };
    let root = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_fixture_root);
    let out = Path::new(&out);
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(err) = fs::create_dir_all(parent) {
                eprintln!("create_dir_all {}: {err}", parent.display());
                return ExitCode::FAILURE;
            }
        }
    }

    let defs = match vic3_defs::load_from_path(&root) {
        Ok(defs) => defs,
        Err(err) => {
            eprintln!("load_from_path {}: {err}", root.display());
            return ExitCode::FAILURE;
        }
    };
    let blob = match vic3_defs::encode_blob(&defs) {
        Ok(blob) => blob,
        Err(err) => {
            eprintln!("encode_blob: {err}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(err) = fs::write(out, &blob) {
        eprintln!("write {}: {err}", out.display());
        return ExitCode::FAILURE;
    }
    eprintln!("wrote {} bytes to {}", blob.len(), out.display());
    ExitCode::SUCCESS
}
