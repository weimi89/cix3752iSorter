/**
 * 執行環境：同一份前端跑在兩個地方，資料來源都是本機的網頁伺服器，差在位址怎麼來。
 *
 * | 環境      | 判斷依據              | API 位址                                  |
 * |-----------|-----------------------|-------------------------------------------|
 * | 桌面 App  | `__TAURI_INTERNALS__` | 啟動時向 Rust 問 `backend_base_url`      |
 * | 瀏覽器    | 沒有上面那個          | 相對路徑（頁面本身就是伺服器給的）        |
 */
const w = typeof window !== 'undefined' ? window : undefined

/** 桌面 App（Tauri webview）內 */
export const isTauriRuntime = !!w?.__TAURI_INTERNALS__

let base = ''

/** API／SSE 的位址前綴；瀏覽器是空字串，桌面模式是 http://127.0.0.1:<埠> */
export const apiBase = () => base

/** 桌面模式在掛載畫面前先問一次；瀏覽器不用 */
export async function initRuntime() {
  if (!isTauriRuntime) return
  const { invoke } = await import('@tauri-apps/api/core')
  base = await invoke('backend_base_url')
}
