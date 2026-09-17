/**
 * 網頁版的登入狀態。
 *
 * 桌面視窗從本機打後端，後端把本機當內網直接放行，永遠視為已通過；
 * 瀏覽器開網頁時要問後端一次：內網來源（現場電腦、手機）後端也放行，
 * 只有從外網連進來的人需要輸入共用密碼。
 */
import { ref } from 'vue'
import { apiBase, isTauriRuntime } from '@/api/runtime'

/** 這條連線能不能存取資料 */
const authenticated = ref(isTauriRuntime)
/** 來源是否被判定為內網（內網免登入） */
const isLan = ref(isTauriRuntime)
/** 後端是否已設定共用密碼 —— 沒設定的話外網再怎麼試都進不來 */
const passwordSet = ref(false)
/** 後端是否對外開放；關閉時外網來源連登入頁都拿不到（後端回 403） */
const enabled = ref(true)
/** 後端連得上嗎。連不上與「沒登入」是兩回事，混在一起會把人誤導到登入頁 */
const reachable = ref(true)
const checked = ref(isTauriRuntime)

/**
 * 向後端確認目前身分，回傳「能不能存取」。
 *
 * **連不上後端時不會把人標成未登入** —— 程式重啟期間內網使用者本來免登入，
 * 若因為一次請求失敗就把他們丟去登入頁，畫面會變成「要密碼、但其實沒有密碼」的死路。
 */
async function refresh() {
  if (isTauriRuntime) return true
  try {
    const res = await fetch(apiBase() + '/auth/status', { credentials: 'same-origin' })
    if (res.status === 403) {
      // 後端明確拒絕（對外開關關閉），這是答案不是故障
      reachable.value = true
      enabled.value = false
      authenticated.value = false
      checked.value = true
      return false
    }
    if (!res.ok) throw new Error(`狀態 ${res.status}`)
    const s = await res.json()
    reachable.value = true
    enabled.value = !!s.enabled
    authenticated.value = !!s.authenticated
    isLan.value = !!s.lan
    passwordSet.value = !!s.password_set
    checked.value = true
    return authenticated.value
  } catch {
    reachable.value = false
    return authenticated.value
  }
}

async function login(password) {
  const res = await fetch(apiBase() + '/auth/login', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ password }),
    credentials: 'same-origin',
  })
  if (!res.ok) {
    let message = `登入失敗（${res.status}）`
    try {
      const b = await res.json()
      if (b?.error) message = b.error
      // 登入頁開著的時候被關掉「開放外部連線」：切到「未對外開放」的說明，而不是當成密碼錯
      if (b?.code === 'not_public') enabled.value = false
    } catch { /* 回應不是 JSON，沿用狀態碼訊息 */ }
    throw new Error(message)
  }
  authenticated.value = true
  reachable.value = true
  return true
}

async function logout() {
  if (isTauriRuntime) return
  try {
    await fetch(apiBase() + '/auth/logout', { method: 'POST', credentials: 'same-origin' })
  } finally {
    authenticated.value = false
  }
}

/** 資料請求收到 401 時呼叫：標記為未登入，讓路由守衛把人帶去登入頁 */
function markUnauthenticated() {
  authenticated.value = false
}

export const useWebAuth = () => ({
  authenticated,
  isLan,
  passwordSet,
  enabled,
  reachable,
  checked,
  refresh,
  login,
  logout,
  markUnauthenticated,
})
