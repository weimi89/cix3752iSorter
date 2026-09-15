/**
 * 功能級別 2FA Composable
 *
 * 提供功能級別的 Authenticator 兩步驟驗證功能：
 * - 每個功能有獨立的 2FA 設定
 * - 用戶需要為每個功能單獨設定 Authenticator
 * - 解鎖狀態會根據後端 session 有效期自動過期
 */

import axios from 'axios'
import { usePage } from '@inertiajs/vue3'

/** 全域解鎖狀態追蹤（按功能 ID） */
const unlockedFeatures = reactive(new Map())

/** 過期定時器追蹤（按功能 ID） */
const expirationTimers = new Map()

/** 是否已初始化 */
let initialized = false

/**
 * 設定功能過期定時器
 */
const setExpirationTimer = (featureId, expiresAt) => {
  // 清除舊的定時器
  if (expirationTimers.has(featureId)) {
    clearTimeout(expirationTimers.get(featureId))
  }

  const now = Date.now()
  const delay = expiresAt - now

  if (delay > 0) {
    const timer = setTimeout(() => {
      unlockedFeatures.delete(featureId)
      expirationTimers.delete(featureId)
    }, delay)

    expirationTimers.set(featureId, timer)
  }
}

/**
 * 初始化已驗證狀態（從後端 props）
 */
const initFromProps = () => {
  if (initialized) return

  try {
    const page = usePage()
    const verified = page.props.feature_2fa_verified || []

    verified.forEach(({ feature_id, expires_at }) => {
      const now = Date.now()
      if (expires_at > now) {
        unlockedFeatures.set(feature_id, true)
        setExpirationTimer(feature_id, expires_at)
      }
    })

    initialized = true
  } catch (e) {
    // 可能在非 Inertia 環境下，忽略錯誤
  }
}

/**
 * 功能級別 2FA Composable
 *
 * @returns {Object} 2FA 相關方法和狀態
 */
export const useFeature2FA = () => {
  // 初始化已驗證狀態
  initFromProps()

  /** 顯示設定對話框 */
  const showSetupDialog = ref(false)

  /** 顯示驗證對話框 */
  const showVerifyDialog = ref(false)

  /** 當前操作的功能 ID */
  const currentFeatureId = ref('')

  /** 當前操作的功能名稱 */
  const currentFeatureName = ref('')

  /** 待執行的操作 */
  const pendingAction = ref(null)

  /**
   * 檢查功能是否已啟用 2FA
   *
   * @param {string} featureId 功能識別碼
   * @returns {Promise<boolean>} 是否已啟用
   */
  const checkFeature2FAEnabled = async (featureId) => {
    try {
      const response = await axios.get(route('feature-2fa.status'), {
        params: { feature_id: featureId },
      })
      return response.data.enabled
    } catch (err) {
      console.error('檢查功能 2FA 狀態失敗:', err)
      return false
    }
  }

  /**
   * 檢查功能是否已在有效期內驗證過（後端 session）
   *
   * @param {string} featureId 功能識別碼
   * @returns {Promise<Object>} { enabled, verified, requires_verification, ttl_minutes }
   */
  const checkFeatureVerified = async (featureId) => {
    try {
      const response = await axios.get(route('feature-2fa.check-verified'), {
        params: { feature_id: featureId },
      })
      return response.data
    } catch (err) {
      console.error('檢查功能驗證狀態失敗:', err)
      return { enabled: false, verified: false, requires_verification: false }
    }
  }

  /**
   * 檢查功能是否已解鎖（本次 session）
   *
   * @param {string} featureId 功能識別碼
   * @returns {boolean} 是否已解鎖
   */
  const isFeatureUnlocked = (featureId) => {
    return unlockedFeatures.get(featureId) === true
  }

  /**
   * 開啟功能 2FA 設定對話框
   *
   * @param {string} featureId 功能識別碼
   * @param {string} featureName 功能名稱
   */
  const openSetupDialog = (featureId, featureName) => {
    currentFeatureId.value = featureId
    currentFeatureName.value = featureName
    showSetupDialog.value = true
  }

  /**
   * 執行需要 2FA 驗證的功能操作
   *
   * 流程：
   * 1. 檢查後端 session 是否已在有效期內驗證過
   * 2. 若已驗證，直接執行操作
   * 3. 若未啟用 2FA，開啟設定對話框
   * 4. 若已啟用但未驗證（或已過期），開啟驗證對話框
   *
   * @param {string} featureId 功能識別碼
   * @param {string} featureName 功能名稱
   * @param {Function} action 要執行的操作
   * @param {Object} options 選項
   * @param {boolean} options.forceVerify 強制重新驗證
   * @returns {Promise} 操作結果
   */
  const requireFeature2FA = async (featureId, featureName, action, options = {}) => {
    const { forceVerify = false } = options

    // 檢查後端 session 的驗證狀態
    const status = await checkFeatureVerified(featureId)

    // 如果已在有效期內驗證過且不強制驗證，直接執行
    if (status.verified && !forceVerify) {
      unlockedFeatures.set(featureId, true)
      return await action()
    }

    if (!status.enabled) {
      // 未啟用 2FA，開啟設定對話框
      return new Promise((resolve, reject) => {
        pendingAction.value = { action, resolve, reject }
        currentFeatureId.value = featureId
        currentFeatureName.value = featureName
        showSetupDialog.value = true
      })
    }

    // 已啟用 2FA 但需要驗證，開啟驗證對話框
    return new Promise((resolve, reject) => {
      pendingAction.value = { action, resolve, reject }
      currentFeatureId.value = featureId
      currentFeatureName.value = featureName
      showVerifyDialog.value = true
    })
  }

  /**
   * 2FA 設定完成後的回調
   *
   * @param {number} ttlMinutes 有效期（分鐘），由後端返回
   */
  const onSetupEnabled = async (ttlMinutes = 10) => {
    const featureId = currentFeatureId.value

    // 設定完成後，標記為已解鎖
    unlockedFeatures.set(featureId, true)

    // 設定過期定時器
    const expiresAt = Date.now() + (ttlMinutes * 60 * 1000)
    setExpirationTimer(featureId, expiresAt)

    if (pendingAction.value) {
      const { action, resolve, reject } = pendingAction.value
      pendingAction.value = null

      try {
        const result = await action()
        resolve(result)
      } catch (err) {
        reject(err)
      }
    }
  }

  /**
   * 2FA 設定取消後的回調
   */
  const onSetupCancelled = () => {
    if (pendingAction.value) {
      const { reject } = pendingAction.value
      pendingAction.value = null
      reject(new Error('2FA 設定已取消'))
    }
  }

  /**
   * 2FA 驗證成功後的回調
   *
   * @param {number} ttlMinutes 有效期（分鐘），由後端返回
   */
  const onVerified = async (ttlMinutes = 10) => {
    const featureId = currentFeatureId.value

    // 驗證成功，標記為已解鎖
    unlockedFeatures.set(featureId, true)

    // 設定過期定時器
    const expiresAt = Date.now() + (ttlMinutes * 60 * 1000)
    setExpirationTimer(featureId, expiresAt)

    if (pendingAction.value) {
      const { action, resolve, reject } = pendingAction.value
      pendingAction.value = null

      try {
        const result = await action()
        resolve(result)
      } catch (err) {
        reject(err)
      }
    }
  }

  /**
   * 2FA 驗證取消後的回調
   */
  const onVerifyCancelled = () => {
    if (pendingAction.value) {
      const { reject } = pendingAction.value
      pendingAction.value = null
      reject(new Error('2FA 驗證已取消'))
    }
  }

  /**
   * 重置功能的解鎖狀態
   *
   * @param {string|null} featureId 功能識別碼，不傳則重置全部
   */
  const resetUnlock = (featureId = null) => {
    if (featureId) {
      unlockedFeatures.delete(featureId)
    } else {
      unlockedFeatures.clear()
    }
  }

  return {
    // 狀態
    showSetupDialog,
    showVerifyDialog,
    currentFeatureId,
    currentFeatureName,
    unlockedFeatures,

    // 方法
    checkFeature2FAEnabled,
    checkFeatureVerified,
    isFeatureUnlocked,
    openSetupDialog,
    requireFeature2FA,
    onSetupEnabled,
    onSetupCancelled,
    onVerified,
    onVerifyCancelled,
    resetUnlock,
  }
}
