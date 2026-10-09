/**
 * 目前持有的更新物件（Tauri updater 的 `Update`）。每個物件在 Rust 端佔一份資源，JS 回收不會釋放；
 * 每小時檢查一次的話，現場遲遲不按安裝就一直累積到程式關閉。換掉或清空時釋放前一個
 * （同一個物件重設不釋放，還在用）。釋放失敗只記錄，不影響更新流程
 */
export function createUpdateHolder() {
  let current = null
  return {
    get: () => current,
    set: next => {
      const prev = current
      current = next
      if (prev && prev !== next) prev.close().catch(e => console.warn('[updater] close failed:', e))
    },
  }
}
