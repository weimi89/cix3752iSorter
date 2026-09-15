/**
 * 自動更新（對齊 cix3752iLabelPrint 的 useUpdater）：Tauri updater 外掛讀 GitHub Release 的 latest.json
 * （minisign 簽章）→ 下載 .deb 安裝 → relaunch。只有桌面視窗有這條；用瀏覽器開後台的人看不到更新。
 *
 * 啟動後 5 秒才做第一次檢查，避開首屏。檢查失敗（沒外網、GitHub 不通）只記 console，
 * 不打擾現場；畫面上只有「真的有新版」才會出現徽章與對話框。
 */
import { ref } from 'vue'
import { isTauriRuntime } from '@/api/runtime'

const updateAvailable = ref(false)
const updateInfo = ref(null) // { version, currentVersion, notes, date }
const isChecking = ref(false)
const isDownloading = ref(false)
const downloadProgress = ref(0)
const stage = ref('') // downloading / installing / restarting
const lastError = ref(null)
let wired = false
let tauriUpdate = null // Tauri updater 的 Update 物件，下載階段要用

export const checkForUpdates = async () => {
  if (!isTauriRuntime || isChecking.value) return
  isChecking.value = true
  try {
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
  } catch (e) {
    console.warn('[updater] check failed:', e)
  } finally {
    isChecking.value = false
  }
}

export const downloadAndInstall = async () => {
  if (isDownloading.value || !tauriUpdate) return
  isDownloading.value = true
  downloadProgress.value = 0
  stage.value = 'downloading'
  lastError.value = null
  try {
    let downloaded = 0
    let total = 0
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
  } catch (e) {
    lastError.value = e?.message || String(e)
    isDownloading.value = false
    stage.value = ''
  }
}

export const dismissUpdate = () => { updateAvailable.value = false }

export const useUpdater = () => {
  if (!wired) {
    wired = true
    if (isTauriRuntime) setTimeout(() => { checkForUpdates() }, 5000)
  }
  return { updateAvailable, updateInfo, isChecking, isDownloading, downloadProgress, stage, lastError, checkForUpdates, downloadAndInstall, dismissUpdate }
}
