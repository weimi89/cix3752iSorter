fn main() {
    // 網頁後台由 rust-embed 於編譯期嵌入 `../dist`。該目錄是 Vite 的產物、
    // 不入版控，乾淨簽出後直接 `cargo check` 會因為找不到目錄而編譯失敗 ——
    // 這裡補一個佔位，讓「只檢查 Rust」的流程不必先跑前端建置。
    // 正式打包時 tauri.conf.json 的 beforeBuildCommand 會先產出真正的 dist。
    let dist = std::path::Path::new("../dist");
    if !dist.exists() {
        let _ = std::fs::create_dir_all(dist);
        let _ = std::fs::write(
            dist.join("index.html"),
            "<!doctype html><meta charset=\"utf-8\"><title>尚未建置</title>\
             <p>網頁後台尚未建置，請先執行 <code>yarn build</code>。",
        );
    }

    // 前端重新建置後要讓 Rust 這側跟著重編，否則 binary 內嵌的還是上一版畫面
    println!("cargo:rerun-if-changed=../dist");

    tauri_build::build()
}
