use std::path::PathBuf;

fn main() {
    // `tauri::generate_context!()` requires `frontendDist` to exist at compile
    // time. `web/dist` is gitignored, so clippy/check and fresh checkouts need a
    // placeholder until `npm run build:desktop` (or `beforeBuildCommand`) runs.
    let dist = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/dist");
    let index = dist.join("index.html");
    println!("cargo:rerun-if-changed={}", index.display());
    if !index.exists() {
        std::fs::create_dir_all(&dist).expect("create web/dist stub for Tauri");
        std::fs::write(
            &index,
            concat!(
                "<!doctype html><html lang=\"en\"><head>",
                "<meta charset=\"utf-8\"/>",
                "<title>Victoria 3 Analyzer</title>",
                "</head><body><div id=\"root\"></div></body></html>\n",
            ),
        )
        .expect("write web/dist/index.html stub");
    }

    tauri_build::build()
}
