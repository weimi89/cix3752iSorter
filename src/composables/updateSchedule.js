/**
 * 檢查更新的排程：啟動後 `firstMs` 檢查一次，之後每 `everyMs` 再檢查。
 *
 * 只在啟動時查一次的話，工控機連開好幾天都不會看到新版；開機剛登入時網路常還沒好，
 * 那一次失敗就再也不查。檢查只會亮起「有新版本」的提示，要有人按才安裝。
 * 計時函式可換掉，測試用假時鐘驗證
 */
export const UPDATE_CHECK_FIRST_MS = 5000
export const UPDATE_CHECK_EVERY_MS = 60 * 60 * 1000

export function startUpdateChecks(check, { firstMs = UPDATE_CHECK_FIRST_MS, everyMs = UPDATE_CHECK_EVERY_MS, setTimeoutFn = setTimeout, setIntervalFn = setInterval } = {}) {
  setTimeoutFn(check, firstMs)
  setIntervalFn(check, everyMs)
}
