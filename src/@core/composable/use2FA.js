/**
 * 2FA 驗證 Composable（每個功能獨立驗證模式）
 *
 * 功能需要 2FA 保護時：
 * 1. 用戶必須先設定 Authenticator
 * 2. 每個功能獨立驗證，解鎖後可使用
 */

import axios from 'axios'

// 全域解鎖狀態追蹤（按功能 ID）
const unlockedFeatures = reactive(new Map())

export const use2FA = () => {
  const showVerifyDialog = ref(false)
  const showSetupRequiredDialog = ref(false)
  const is2FAEnabled = ref(false)
  const pendingAction = ref(null)
  const pendingFeatureId = ref(null)
  const pendingFeatureName = ref('')

  /**
   * 檢查用戶是否啟用 2FA
   */
  const check2FAEnabled = async () => {
    try {
      const response = await axios.get(route('2fa.status'))
      is2FAEnabled.value = response.data.enabled
      return response.data.enabled
    } catch (err) {
      console.error('檢查 2FA 狀態失敗:', err)
      return false
    }
  }

  /**
   * 檢查功能是否已解鎖
   *
   * @param {string} featureId - 功能識別碼
   * @returns {boolean} - 是否已解鎖
   */
  const isFeatureUnlocked = (featureId) => {
    return unlockedFeatures.get(featureId) === true
  }

  /**
   * 執行需要 2FA 驗證的操作（每個功能獨立驗證）
   *
   * @param {string} featureId - 功能識別碼（唯一識別此功能）
   * @param {Function} action - 要執行的異步操作
   * @param {Object} options - 選項
   * @param {string} options.featureName - 功能名稱（用於顯示）
   * @param {boolean} options.forceVerify - 強制重新驗證（即使已解鎖）
   * @returns {Promise} - 操作結果
   */
  const require2FA = async (featureId, action, options = {}) => {
    const { featureName = '此功能', forceVerify = false } = options

    // 先檢查是否啟用 2FA
    const enabled = await check2FAEnabled()

    // 如果未啟用 2FA，顯示需要設定的提示
    if (!enabled) {
      pendingFeatureName.value = featureName
      showSetupRequiredDialog.value = true
      return Promise.reject(new Error('需要先設定兩步驟驗證'))
    }

    // 如果此功能已解鎖且不強制重新驗證，直接執行
    if (isFeatureUnlocked(featureId) && !forceVerify) {
      return await action()
    }

    // 需要驗證，返回一個 Promise
    return new Promise((resolve, reject) => {
      pendingAction.value = { action, resolve, reject }
      pendingFeatureId.value = featureId
      pendingFeatureName.value = featureName
      showVerifyDialog.value = true
    })
  }

  /**
   * 驗證成功後的回調
   */
  const onVerified = async () => {
    if (pendingAction.value) {
      const { action, resolve, reject } = pendingAction.value
      const featureId = pendingFeatureId.value

      pendingAction.value = null
      pendingFeatureId.value = null

      // 標記此功能為已解鎖
      if (featureId) {
        unlockedFeatures.set(featureId, true)
      }

      try {
        const result = await action()
        resolve(result)
      } catch (err) {
        reject(err)
      }
    }
  }

  /**
   * 驗證取消後的回調
   */
  const onCancelled = () => {
    if (pendingAction.value) {
      const { reject } = pendingAction.value
      pendingAction.value = null
      pendingFeatureId.value = null
      reject(new Error('2FA 驗證已取消'))
    }
  }

  /**
   * 重置功能的解鎖狀態（登出或需要重新驗證時使用）
   *
   * @param {string} featureId - 功能識別碼，不傳則重置全部
   */
  const resetUnlock = (featureId = null) => {
    if (featureId) {
      unlockedFeatures.delete(featureId)
    } else {
      unlockedFeatures.clear()
    }
  }

  /**
   * 前往 2FA 設定頁面
   */
  const goToSetup = () => {
    showSetupRequiredDialog.value = false
    window.location.href = route('2fa.index')
  }

  /**
   * 關閉設定提示對話框
   */
  const closeSetupDialog = () => {
    showSetupRequiredDialog.value = false
  }

  return {
    showVerifyDialog,
    showSetupRequiredDialog,
    is2FAEnabled,
    unlockedFeatures,
    pendingFeatureName,
    check2FAEnabled,
    isFeatureUnlocked,
    require2FA,
    onVerified,
    onCancelled,
    resetUnlock,
    goToSetup,
    closeSetupDialog,
  }
}
