/**
 * 後端資料通道：所有頁面經這裡打 REST。
 *
 * - 非 2xx 一律 throw，訊息取後端的 `error` 欄位，頁面用 try/catch + toast 呈現
 * - 需要設定密碼的操作（設定、格口表、重置分揀機）自動帶 `X-Settings-Password`
 */
import { getSettingsPassword } from '@/composables/useSettingsPassword'
import { apiBase } from '@/api/runtime'

/** 空字串／null 的查詢參數不送，後端一律當「不篩選」 */
const qs = params => new URLSearchParams(Object.entries(params || {}).filter(([, v]) => v !== '' && v != null && v !== undefined))

async function request(method, path, body, { password = false, raw = false } = {}) {
  const headers = {}
  if (body !== undefined) headers['Content-Type'] = 'application/json'
  if (password) headers['X-Settings-Password'] = getSettingsPassword()
  let res
  try {
    res = await fetch(apiBase() + path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) })
  } catch (e) {
    throw new Error(`無法連線到分揀控制服務：${e.message}`)
  }
  if (!res.ok) {
    let message = `伺服器回應 ${res.status}`
    try {
      const j = await res.json()
      if (j?.error) message = j.error
    } catch { /* 非 JSON 錯誤頁 */ }
    const err = new Error(message)
    err.status = res.status
    throw err
  }
  if (raw) return res
  if (res.status === 204) return null
  return await res.json()
}

export const api = {
  status: () => request('GET', '/api/status'),
  health: () => request('GET', '/api/health'),
  authCheck: password => request('POST', '/api/auth/check', { password }),

  parcels: params => request('GET', `/api/parcels?${qs(params)}`),
  parcel: id => request('GET', `/api/parcels/${id}`),
  parcelsExportUrl: params => `${apiBase()}/api/parcels/export.xlsx?${qs(params)}`,
  printJobPreviewUrl: id => `${apiBase()}/api/print-jobs/${id}/preview.png?t=${Date.now()}`,
  logFiles: () => request('GET', '/api/logs/files'),
  logFileUrl: name => `${apiBase()}/api/logs/files/${encodeURIComponent(name)}`,
  lanIps: () => request('GET', '/api/lan-ips'),
  irStatus: () => request('GET', '/api/ir/status'),
  irDetail: m2 => request('POST', '/api/ir/detail', { m2 }),
  irBlock: (m2, block) => request('POST', '/api/ir/block', { m2, block }, { password: true }),
  dailyStats: (days = 30) => request('GET', `/api/stats/daily?days=${days}`),
  hourlyStats: (hours = 12) => request('GET', `/api/stats/hourly?hours=${hours}`),

  config: () => request('GET', '/api/config'),
  saveConfig: cfg => request('PUT', '/api/config', cfg, { password: true }),
  chutes: () => request('GET', '/api/chutes'),
  saveChutes: list => request('PUT', '/api/chutes', list, { password: true }),

  beltStart: () => request('POST', '/api/belt/start'),
  beltStop: () => request('POST', '/api/belt/stop'),
  sorterReset: () => request('POST', '/api/sorter/reset', undefined, { password: true }),
  sorterCommand: command => request('POST', '/api/sorter/command', { command }, { password: true }),
  deviceTest: (target, addr) => request('POST', '/api/devices/test', { target, addr }),

  printJobs: params => request('GET', `/api/print-jobs?${qs(params)}`),
  printJobRetry: id => request('POST', `/api/print-jobs/${id}/retry`),
  printers: () => request('GET', '/api/printers'),
  printerTest: port => request('POST', `/api/printers/${encodeURIComponent(port)}/test`),

  reportQueue: params => request('GET', `/api/report-queue?${qs(params)}`),
  reportRetry: id => request('POST', `/api/report-queue/${id}/retry`),

  logs: params => request('GET', `/api/logs?${qs(params)}`),

  updateStatus: () => request('GET', '/api/update/status'),
  updateCheck: () => request('POST', '/api/update/check'),
  updateInstall: () => request('POST', '/api/update/install', undefined, { password: true }),
  updateUpload: async file => {
    const res = await fetch(apiBase() + '/api/update/upload', { method: 'POST', headers: { 'X-Settings-Password': getSettingsPassword(), 'Content-Type': 'application/gzip' }, body: file })
    if (!res.ok) { let m = `伺服器回應 ${res.status}`; try { m = (await res.json()).error || m } catch {} const e = new Error(m); e.status = res.status; throw e }
    return res.json()
  },
}
