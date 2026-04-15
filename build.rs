//! Build script — ensures ui/dist/ exists so rust-embed compiles without
//! requiring a separate `npm run build` step. If the UI hasn't been built,
//! a placeholder page is embedded instead.

use std::path::Path;

fn main() {
    let dist = Path::new("ui/dist");
    if !dist.join("index.html").exists() {
        std::fs::create_dir_all(dist).expect("failed to create ui/dist");
        std::fs::write(
            dist.join("index.html"),
            r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Dolphin Milk</title>
<style>
  body { font-family: system-ui; background: #1b2431; color: #fff; display: flex;
         justify-content: center; align-items: center; height: 100vh; margin: 0; }
  .box { text-align: center; max-width: 480px; }
  code { background: #273142; padding: 2px 8px; border-radius: 4px; }
  a { color: #4880ff; }
</style></head>
<body><div class="box">
  <h2>Web UI not built yet</h2>
  <p>Run <code>cd ui && npm install && npm run build</code> then restart the server.</p>
  <p>The API is fully functional — use <a href="/health">/health</a> or <a href="/agent">/agent</a>.</p>
</div></body></html>
"#,
        )
        .expect("failed to write placeholder index.html");
    }

    // Tell Cargo to rerun if the dist folder changes
    println!("cargo:rerun-if-changed=ui/dist/index.html");
}
