/**
 * 事件訂閱（SSE）。所有頁面共用同一條 `/events/stream` 連線，在瀏覽器端分發；
 * 每頁各開一條會撞到瀏覽器同網域連線數上限，症狀是後開的頁面收不到事件。
 *
 *     const unlisten = listen('parcel-updated', evt => { ... })
 */
import { apiBase } from '@/api/runtime'

const subscribers = new Map()
let source = null
let retryDelay = 1000
let retryTimer = null
const connectionListeners = new Set()
let connected = false

function setConnected(v) {
  if (connected === v) return
  connected = v
  for (const cb of connectionListeners) cb(v)
}

function dispatch(name, payload) {
  const set = subscribers.get(name)
  if (!set) return
  for (const cb of [...set]) {
    try { cb({ event: name, payload }) } catch (e) { console.error(`[events] ${name} 處理函式拋出例外`, e) }
  }
}

function ensureSource() {
  if (source || typeof EventSource === 'undefined' || retryTimer) return
  source = new EventSource(apiBase() + '/events/stream')
  source.onopen = () => { retryDelay = 1000; setConnected(true) }
  source.onmessage = e => {
    try {
      const { event, payload } = JSON.parse(e.data)
      dispatch(event, payload)
    } catch (err) {
      console.error('[events] 事件解析失敗', err, e.data)
    }
  }
  source.onerror = () => {
    source?.close()
    source = null
    setConnected(false)
    retryTimer = setTimeout(() => {
      retryTimer = null
      if (subscribers.size) ensureSource()
    }, retryDelay)
    retryDelay = Math.min(retryDelay * 2, 15000)
  }
}

export function listen(name, cb) {
  if (!subscribers.has(name)) subscribers.set(name, new Set())
  subscribers.get(name).add(cb)
  ensureSource()
  return () => {
    subscribers.get(name)?.delete(cb)
    if (subscribers.get(name)?.size === 0) subscribers.delete(name)
  }
}

/** 連線狀態變化（頂部顯示「與後端斷線」用） */
export function onConnection(cb) {
  connectionListeners.add(cb)
  cb(connected)
  return () => connectionListeners.delete(cb)
}
