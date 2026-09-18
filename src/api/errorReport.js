/**
 * 把畫面上的 JavaScript 錯誤送回後端記進事件記錄（`ui` 類別）。
 *
 * 正式版桌面沒有開發者工具，元件出錯只會默默不顯示，現場回報時完全沒有線索；
 * 有了這條，事件記錄頁就查得到錯誤訊息、堆疊、出事的頁面與環境（桌面／網頁）。
 *
 * 三個來源都接：Vue 元件錯誤（setup／render／watcher）、未捕捉的 window error、未處理的 Promise 拒絕。
 * 同一則訊息 30 秒內只送一次、每分鐘最多 20 則；回報本身失敗時不再回報，避免陷入迴圈。
 */
import { api } from '@/api/http'
import { isTauriRuntime } from '@/api/runtime'

const DEDUP_MS = 30_000
const MAX_PER_MINUTE = 20
const recent = new Map()
let windowStart = 0
let windowCount = 0

const describe = err => {
  if (err instanceof Error) return { message: err.message || String(err), stack: err.stack || '' }
  if (typeof err === 'string') return { message: err, stack: '' }
  try { return { message: JSON.stringify(err), stack: '' } } catch { return { message: String(err), stack: '' } }
}

const report = (err, source, router) => {
  const { message, stack } = describe(err)
  const now = Date.now()
  const key = `${source}|${message}`
  if ((recent.get(key) || 0) > now - DEDUP_MS) return
  recent.set(key, now)
  if (now - windowStart > 60_000) { windowStart = now; windowCount = 0 }
  if (++windowCount > MAX_PER_MINUTE) return
  api.reportClientError({
    message,
    stack,
    source,
    page: router?.currentRoute?.value?.fullPath || location.hash || '',
    runtime: isTauriRuntime ? 'desktop' : 'web',
    ua: navigator.userAgent,
  }).catch(() => { /* 回報失敗（後端重啟中、離線）就算了，不能再回報自己 */ })
}

export function installErrorReporting(app, router) {
  app.config.errorHandler = (err, instance, info) => {
    // 設了 errorHandler 之後 Vue 就不再自己印，這裡補印才不會讓瀏覽器主控台變啞
    console.error(err)
    const name = instance?.$options?.name || instance?.$?.type?.__name || instance?.$?.type?.name || ''
    report(err, `vue:${info}${name ? ` <${name}>` : ''}`, router)
  }
  window.addEventListener('error', e => {
    report(e.error || e.message, `window:${(e.filename || '').split('/').pop()}:${e.lineno || 0}`, router)
  })
  window.addEventListener('unhandledrejection', e => {
    report(e.reason, 'promise', router)
  })
}
