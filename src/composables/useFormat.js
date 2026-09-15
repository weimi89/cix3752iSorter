/** 顯示用的格式化：時間、狀態文字與顏色。狀態代碼沿用舊系統 1–8。 */
export const STATUS = {
  1: { key: 'status.init', color: 'secondary' },
  2: { key: 'status.received', color: 'info' },
  3: { key: 'status.done', color: 'success' },
  4: { key: 'status.lost', color: 'error' },
  5: { key: 'status.blocked', color: 'warning' },
  6: { key: 'status.blockedTaken', color: 'error' },
  7: { key: 'status.cancelled', color: 'error' },
  8: { key: 'status.triggerNg', color: 'error' },
}

export const SOURCE = {
  pending: { key: 'source.pending', color: 'secondary' },
  api: { key: 'source.api', color: 'success' },
  timeout: { key: 'source.timeout', color: 'warning' },
  noread: { key: 'source.noread', color: 'warning' },
  default: { key: 'source.default', color: 'error' },
  manual: { key: 'source.manual', color: 'info' },
}

export function statusMeta(code) {
  return STATUS[code] || { key: 'status.unknown', color: 'secondary' }
}

export function sourceMeta(src) {
  return SOURCE[src] || { key: 'source.pending', color: 'secondary' }
}

const pad = n => String(n).padStart(2, '0')

/** epoch 毫秒 → `MM-DD HH:mm:ss.SSS` */
export function fmtMs(ms) {
  if (!ms) return ''
  const d = new Date(ms)
  return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${String(d.getMilliseconds()).padStart(3, '0')}`
}

export function fmtDuration(ms) {
  if (ms == null) return ''
  if (ms < 1000) return `${ms} ms`
  return `${(ms / 1000).toFixed(1)} s`
}

export function fmtDate(d) {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}
