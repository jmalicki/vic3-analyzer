//! Emit a postcard defs blob from the in-repo fixture tree for local development
//! and tests. It stays out of `web/public/` so production builds ship no demo
//! definitions; a deployed site has none until the user builds their own.
//!
//! Usage:
//! ```text
//! cargo run -p vic3-defs --bin emit_fixture_blob -- web/fixtures/defs.postcard
//! ```

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn main() -> ExitCode {
    let Some(out) = env::args().nth(1) else {
        eprintln!("usage: emit_fixture_blob <output.postcard>");
        return ExitCode::FAILURE;
    };
    let out = Path::new(&out);
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(err) = fs::create_dir_all(parent) {
                eprintln!("create_dir_all {}: {err}", parent.display());
                return ExitCode::FAILURE;
            }
        }
    }

    let root = fixture_root();
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
