/**
 * 即時狀態：首查 `/api/status`，之後靠 SSE `status` / `device-state` 事件更新（每 0.5 秒一次）。
 */
import { defineStore } from 'pinia'
import { api } from '@/api/http'
import { listen, onConnection } from '@/api/events'

export const useStatusStore = defineStore('status', {
  state: () => ({
    version: '',
    devices: { belt: { connected: false }, sorter: { connected: false }, camera: { connected: false } },
    tracker: null,
    print: { pending: 0, failed: 0 },
    report: { pending: 0, failed: 0 },
    chuteLatency: null,
    sseConnected: false,
    lastRefreshAt: null,
    _unlisten: [],
  }),
  getters: {
    allDevicesUp: s => s.devices.belt.connected && s.devices.sorter.connected && s.devices.camera.connected,
    todayCount: s => s.tracker?.today_count ?? 0,
    beltRunning: s => !!s.tracker?.belt_running,
  },
  actions: {
    async refresh() {
      try {
        const d = await api.status()
        this.version = d.version
        this.devices = d.devices
        this.tracker = d.tracker
        this.print = d.print
        this.report = d.report
        this.chuteLatency = d.chute_latency ?? null
        this.lastRefreshAt = Date.now()
      } catch (e) {
        console.warn('狀態載入失敗', e)
      }
    },
    start() {
      if (this._unlisten.length) return
      this.refresh()
      this._unlisten.push(
        listen('status', ({ payload }) => { this.tracker = payload }),
        listen('device-state', ({ payload }) => {
          const slot = this.devices[payload.device]
          if (slot) { slot.connected = payload.connected; slot.since_ms = payload.ts_ms }
        }),
        listen('print-job', () => { this.refreshCountsSoon() }),
        listen('report-queue', () => { this.refreshCountsSoon() }),
        onConnection(v => { this.sseConnected = v; if (v) this.refresh() }),
      )
      // 兜底：SSE 掉了也每 10 秒補一次
      this._timer = setInterval(() => this.refresh(), 10000)
    },
    refreshCountsSoon() {
      clearTimeout(this._countsTimer)
      this._countsTimer = setTimeout(() => this.refresh(), 800)
    },
    stop() {
      for (const u of this._unlisten) u()
      this._unlisten = []
      clearInterval(this._timer)
    },
  },
})
