//! 網頁後台的靜態出口：`../dist`（Vite 產物）於編譯期嵌入 binary（見 `build.rs`），
//! 安裝後不必另外部署靜態檔。

use axum::{
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use once_cell::sync::Lazy;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../dist"]
struct Assets;

fn build_index_html() -> String {
    Assets::get("index.html")
        .map(|f| String::from_utf8_lossy(f.data.as_ref()).into_owned())
        .unwrap_or_else(|| "<!doctype html><meta charset=\"utf-8\"><p>網頁後台尚未建置。".to_string())
}

static INDEX_HTML: Lazy<String> = Lazy::new(build_index_html);

/// 開發模式每次重讀：Vite 產物檔名帶內容雜湊，前端一重建就換一組；
/// 沿用啟動時快取的 index.html 會指向已不存在的 JS，落到 SPA fallback 拿回 HTML，
/// 瀏覽器把 HTML 當 JS 執行 → 整頁白畫面而且看不出錯誤。
fn index_html() -> String {
    if cfg!(debug_assertions) { build_index_html() } else { INDEX_HTML.clone() }
}

/// `assets/` 檔名帶雜湊可鎖一年；其餘檔名固定、內容可能換，只給一小時。
fn cache_control_for(path: &str) -> &'static str {
    if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else if path.ends_with(".woff2") || path.ends_with(".woff") || path.ends_with(".ttf") {
        "public, max-age=2592000"
    } else {
        "public, max-age=3600"
    }
}

fn index_response() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        index_html(),
    )
        .into_response()
}

/// 靜態資源與 SPA fallback：找不到檔案回 index.html，前端路由重新整理才不會 404。
pub(super) async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.is_empty() || path == "index.html" {
        return index_response();
    }
    match Assets::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, mime.as_ref()),
                    (header::CACHE_CONTROL, cache_control_for(path)),
                ],
                file.data.into_owned(),
            )
                .into_response()
        }
        None => index_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_指向的資產都取得到() {
        let html = index_html();
        for prefix in ["src=\"/assets/", "href=\"/assets/"] {
            let mut rest = html.as_str();
            while let Some(i) = rest.find(prefix) {
                rest = &rest[i + prefix.len()..];
                let Some(end) = rest.find('"') else { break };
                let file = &rest[..end];
                assert!(
                    Assets::get(&format!("assets/{file}")).is_some(),
                    "index.html 指向 assets/{file}，但檔案不存在 —— 網頁會白畫面"
                );
                rest = &rest[end..];
            }
        }
    }

    #[test]
    fn 只有帶雜湊的產物可以長快取() {
        assert!(cache_control_for("assets/index-abc123.js").contains("immutable"));
        assert!(!cache_control_for("favicon.ico").contains("immutable"));
    }
}
