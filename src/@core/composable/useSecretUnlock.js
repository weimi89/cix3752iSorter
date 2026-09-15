import { useTimeoutFn } from '@vueuse/core'

// 隱藏彩蛋解鎖狀態（全域共享）
const isSecretUnlocked = ref(false)
const SECRET_CODE = 'PIG'
const TAP_COUNT_REQUIRED = 10
const AUTO_LOCK_MINUTES = 10

// 鍵盤輸入緩衝區
const inputBuffer = ref('')

// 點擊計數器
const tapCount = ref(0)
const tapTimeout = ref(null)

// 自動鎖定計時器
const { start: startAutoLock, stop: stopAutoLock } = useTimeoutFn(() => {
  isSecretUnlocked.value = false
}, AUTO_LOCK_MINUTES * 60 * 1000, { immediate: false })

// 音效
const pigSound = ref(null)
const unlockSound = ref(null)

// 初始化音效
const initSounds = () => {
  if (typeof window !== 'undefined' && !pigSound.value) {
    pigSound.value = new Audio('/sounds/pig-oink.mp3')
    pigSound.value.volume = 0.5
    unlockSound.value = new Audio('/sounds/unlock-success.mp3')
    unlockSound.value.volume = 0.6
  }
}

// 播放豬叫聲
const playPigSound = () => {
  initSounds()
  if (pigSound.value) {
    pigSound.value.currentTime = 0
    pigSound.value.play().catch(() => {})
  }
}

// 播放解鎖音效
const playUnlockSound = () => {
  initSounds()
  if (unlockSound.value) {
    unlockSound.value.currentTime = 0
    unlockSound.value.play().catch(() => {})
  }
}

// 鎖定函數
const lock = () => {
  isSecretUnlocked.value = false
  stopAutoLock()
}

// 解鎖函數
const unlock = () => {
  isSecretUnlocked.value = true
  playUnlockSound()

  // 重新啟動自動鎖定計時器（10 分鐘後自動鎖定）
  stopAutoLock()
  startAutoLock()
}

// 處理鍵盤輸入
const handleKeyPress = (event, hasSecretItems) => {
  // 如果沒有隱藏項目，不需要處理
  if (!hasSecretItems) {
    return
  }

  // 忽略在輸入框中的按鍵
  if (event.target.tagName === 'INPUT' || event.target.tagName === 'TEXTAREA') {
    return
  }

  // 只接受字母鍵
  const key = event.key.toUpperCase()
  if (!/^[A-Z]$/.test(key)) {
    return
  }

  // 累積輸入
  inputBuffer.value += key

  // 只保留最近 N 個字元（N = 密碼長度）
  if (inputBuffer.value.length > SECRET_CODE.length) {
    inputBuffer.value = inputBuffer.value.slice(-SECRET_CODE.length)
  }

  // 檢查是否匹配
  if (inputBuffer.value === SECRET_CODE) {
    unlock()
    inputBuffer.value = ''
  }
}

// 處理點擊（用於手機）
const handleTap = (hasSecretItems) => {
  // 清除之前的 timeout
  if (tapTimeout.value) {
    clearTimeout(tapTimeout.value)
  }

  // 增加點擊計數
  tapCount.value++

  // 播放豬叫聲
  playPigSound()

  // 檢查是否達到要求次數（只有在有隱藏項目且尚未解鎖時才觸發解鎖）
  if (hasSecretItems && !isSecretUnlocked.value && tapCount.value >= TAP_COUNT_REQUIRED) {
    unlock()
    tapCount.value = 0
  }

  // 2 秒內沒有繼續點擊就重置計數
  tapTimeout.value = setTimeout(() => {
    tapCount.value = 0
  }, 2000)
}

// Composable 導出
export const useSecretUnlock = () => {
  return {
    isSecretUnlocked: readonly(isSecretUnlocked),
    handleKeyPress,
    handleTap,
    tapCount: readonly(tapCount),
    TAP_COUNT_REQUIRED,
  }
}
