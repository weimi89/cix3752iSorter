/**
 * 自動更新（對齊 cix3752iLabelPrint 的 useUpdater）。同一份畫面，兩條路：
 *
 * - 桌面 App：Tauri updater 外掛讀 GitHub Release 的 latest.json（minisign 簽章）→ 下載安裝 → relaunch
 * - 瀏覽器（headless 服務）：問後端 `/api/update/status`（後端自己會定期查），安裝由後端換執行檔後
 *   交給 supervisor／systemd 重啟，前端輪詢 /api/health 等新版起來再整頁重載；
 *   沒外網也可在同一對話框上傳 tar.gz
 *
 * 啟動後 5 秒才做第一次檢查，避開首屏。檢查失敗（沒外網、GitHub 不通）只記 console，
 * 不打擾現場；畫面上只有「真的有新版」才會出現徽章與對話框。
 */
import { ref } from 'vue'
import { api } from '@/api/http'
import { listen } from '@/api/events'
import { isTauriRuntime } from '@/api/runtime'
import { useSettingsPassword } from '@/composables/useSettingsPassword'

const updateAvailable = ref(false)
const updateInfo = ref(null) // { version, currentVersion, notes, date }
const isChecking = ref(false)
const isDownloading = ref(false)
const downloadProgress = ref(0)
const stage = ref('') // downloading / installing / restarting
const lastError = ref(null)
let wired = false
let tauriUpdate = null // Tauri updater 的 Update 物件，下載階段要用

const applyInfo = info => {
  if (!info?.available) {
    updateAvailable.value = false
    updateInfo.value = null
    return
  }
  updateInfo.value = { version: info.latest, currentVersion: info.current, notes: info.notes || '', date: info.pub_date || '' }
  updateAvailable.value = true
}

const checkTauri = async () => {
  const { check } = await import('@tauri-apps/plugin-updater')
  const update = await check()
  if (!update?.available) {
    tauriUpdate = null
    updateAvailable.value = false
    updateInfo.value = null
    return
  }
  tauriUpdate = update
  updateInfo.value = { version: update.version, currentVersion: update.currentVersion, notes: update.body || '', date: update.date || '' }
  updateAvailable.value = true
}

export const checkForUpdates = async () => {
  if (isChecking.value) return
  isChecking.value = true
  try {
    if (isTauriRuntime) await checkTauri()
    else applyInfo(await api.updateCheck())
  } catch (e) {
    console.warn('[updater] check failed:', e)
  } finally {
    isChecking.value = false
  }
}

const waitForRestart = async () => {
  stage.value = 'restarting'
  // 舊行程 1 秒後結束，supervisor 拉起新版約需 2–5 秒
  await new Promise(r => setTimeout(r, 3000))
  for (let i = 0; i < 60; i++) {
    try {
      const h = await api.health()
      if (h?.ok && h.version !== updateInfo.value?.currentVersion) { location.reload(); return }
    } catch { /* 還沒起來 */ }
    await new Promise(r => setTimeout(r, 1000))
  }
  lastError.value = '服務重啟逾時，請確認 supervisor／systemd 是否有把程式拉起來'
  isDownloading.value = false
}

const installTauri = async () => {
  if (!tauriUpdate) return
  let downloaded = 0
  let total = 0
  stage.value = 'downloading'
  await tauriUpdate.downloadAndInstall(event => {
    switch (event.event) {
      case 'Started':
        total = event.data.contentLength || 0
        break
      case 'Progress':
        downloaded += event.data.chunkLength || 0
        if (total > 0) downloadProgress.value = Math.round((downloaded / total) * 100)
        break
      case 'Finished':
        downloadProgress.value = 100
        stage.value = 'installing'
        break
    }
  })
  stage.value = 'restarting'
  const { relaunch } = await import('@tauri-apps/plugin-process')
  await relaunch()
}

export const downloadAndInstall = async () => {
  if (isDownloading.value) return
  const pw = useSettingsPassword()
  if (!(await pw.ensure())) return
  isDownloading.value = true
  downloadProgress.value = 0
  stage.value = 'downloading'
  lastError.value = null
  try {
    if (isTauriRuntime) {
      await installTauri()
    } else {
      await api.updateInstall()
      await waitForRestart()
    }
  } catch (e) {
    if (e.status === 403) pw.forget()
    lastError.value = e?.message || String(e)
    isDownloading.value = false
    stage.value = ''
  }
}

/** 沒外網時：上傳發版的 tar.gz（只有 headless 服務支援；桌面 App 走系統安裝包） */
export const uploadAndInstall = async file => {
  if (isDownloading.value || !file || isTauriRuntime) return
  const pw = useSettingsPassword()
  if (!(await pw.ensure())) return
  isDownloading.value = true
  downloadProgress.value = 0
  stage.value = 'installing'
  lastError.value = null
  try {
    await api.updateUpload(file)
    await waitForRestart()
  } catch (e) {
    if (e.status === 403) pw.forget()
    lastError.value = e.message
    isDownloading.value = false
    stage.value = ''
  }
}

export const dismissUpdate = () => { updateAvailable.value = false }

export const useUpdater = () => {
  if (!wired) {
    wired = true
    if (isTauriRuntime) {
      setTimeout(() => { checkForUpdates() }, 5000)
    } else {
      listen('update-available', ({ payload }) => applyInfo(payload))
      listen('update-progress', ({ payload }) => { stage.value = payload.stage; downloadProgress.value = payload.percent || 0 })
      setTimeout(async () => {
        try { const s = await api.updateStatus(); applyInfo(s.last) } catch { /* 後端未啟動更新服務 */ }
      }, 5000)
    }
  }
  return { updateAvailable, updateInfo, isChecking, isDownloading, downloadProgress, stage, lastError, checkForUpdates, downloadAndInstall, uploadAndInstall, dismissUpdate, canUpload: !isTauriRuntime }
}
