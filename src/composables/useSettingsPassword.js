/**
 * 設定密碼：改設定、格口表、重置分揀機、更新程式時要帶；記在 sessionStorage，關分頁就忘。
 *
 * 狀態是全站單例：對話框只在 DefaultLayout 掛一次，任何頁面／composable 呼叫 `ensure()`
 * 都會用同一個對話框問密碼。各頁各自 new 一個對話框的話，`ensure()` 會等一個沒人渲染的對話框、永遠不回來。
 */
import { ref, computed } from 'vue'
import { api } from '@/api/http'

const KEY = 'cix-sorter-settings-password'
const password = ref((() => { try { return sessionStorage.getItem(KEY) || '' } catch { return '' } })())
const dialog = ref(false)
const input = ref('')
const error = ref('')
let resolver = null

export function getSettingsPassword() {
  return password.value
}

/** 已有密碼就直接回 true；否則跳對話框問一次 */
const ensure = () => new Promise(resolve => {
  if (password.value) { resolve(true); return }
  if (resolver) resolver(false) // 前一個還在等的請求作廢
  resolver = resolve
  input.value = ''
  error.value = ''
  dialog.value = true
})

const submit = async () => {
  try {
    const r = await api.authCheck(input.value)
    if (!r.ok) { error.value = '密碼錯誤'; return }
  } catch (e) {
    error.value = e.message
    return
  }
  password.value = input.value
  try { sessionStorage.setItem(KEY, input.value) } catch {}
  dialog.value = false
  resolver?.(true)
  resolver = null
}

const cancel = () => {
  dialog.value = false
  resolver?.(false)
  resolver = null
}

const forget = () => {
  password.value = ''
  try { sessionStorage.removeItem(KEY) } catch {}
}

const hasPassword = computed(() => !!password.value)

export function useSettingsPassword() {
  return { dialog, input, error, ensure, submit, cancel, forget, hasPassword }
}
