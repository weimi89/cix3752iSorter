/**
 * 有未儲存的變更時，切換頁面或關閉／重新整理視窗先問一次。
 * 要放在頁面元件的 setup 裡呼叫（路由離開的攔截只認元件本身）。
 */
import { onBeforeRouteLeave } from 'vue-router'
import { useI18n } from 'vue-i18n'

export function useUnsavedGuard(dirty) {
  const { t } = useI18n()
  onBeforeRouteLeave(() => {
    if (dirty.value && !confirm(t('common.leaveConfirm'))) return false
  })
  const beforeUnload = e => {
    if (dirty.value) { e.preventDefault(); e.returnValue = '' }
  }
  onMounted(() => window.addEventListener('beforeunload', beforeUnload))
  onBeforeUnmount(() => window.removeEventListener('beforeunload', beforeUnload))
}
