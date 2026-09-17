<script setup>
/**
 * 外網登入頁：只有從現場網路以外連進來的人會看到（內網與桌面視窗由後端直接放行）。
 * 後端連不上時自動重試 —— 程式重啟期間內網使用者本來免登入，
 * 沒有這段的話他們會卡在一個不需要也填不了密碼的畫面，只能自己想到重新整理。
 */
import { ref, onMounted, onUnmounted, nextTick } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { useWebAuth } from '@/composables/useWebAuth'

const { t } = useI18n()
const router = useRouter()
const route = useRoute()
const { login, refresh, passwordSet, enabled, reachable, authenticated } = useWebAuth()

const password = ref('')
const showPassword = ref(false)
const submitting = ref(false)
const errorMsg = ref('')
const passwordField = ref(null)

const target = () => (typeof route.query.redirect === 'string' ? route.query.redirect : '/')
// 手機遙控頁是獨立的 HTML（不在 SPA 路由裡），登入後要整頁跳過去
const goTarget = async () => {
  const t = target()
  if (t.startsWith('/control')) { window.location.assign(t); return }
  await router.replace(t)
}

let retryTimer = null
let disposed = false

const scheduleRetry = () => {
  if (disposed || retryTimer) return
  retryTimer = setTimeout(async () => {
    retryTimer = null
    if (disposed) return
    await refresh()
    if (disposed) return
    if (authenticated.value) { await goTarget(); return }
    if (!reachable.value) scheduleRetry()
  }, 3000)
}

onMounted(async () => {
  await refresh()
  if (authenticated.value) { await goTarget(); return }
  if (!reachable.value) scheduleRetry()
  await nextTick()
  passwordField.value?.focus?.()
})

onUnmounted(() => {
  disposed = true
  if (retryTimer) { clearTimeout(retryTimer); retryTimer = null }
})

const retryNow = async () => {
  errorMsg.value = ''
  await refresh()
  if (authenticated.value) await goTarget()
}

const submit = async () => {
  if (submitting.value || !password.value) return
  submitting.value = true
  errorMsg.value = ''
  try {
    await login(password.value)
    await goTarget()
  } catch (e) {
    errorMsg.value = e.message
    password.value = ''
    passwordField.value?.focus?.()
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <div class="login-wrap">
    <VCard class="login-card" elevation="8">
      <VCardText class="pa-6 pa-sm-8">
        <div class="text-center mb-6">
          <VIcon :icon="reachable ? 'tabler-lock' : 'tabler-plug-connected-x'" size="48" :color="reachable ? 'primary' : 'warning'" />
          <h1 class="text-h5 font-weight-medium mt-3 mb-1">{{ t('page.login.title') }}</h1>
          <p class="text-body-2 text-medium-emphasis mb-0">
            {{ !reachable ? t('page.login.offlineSubtitle') : (enabled ? t('page.login.subtitle') : t('page.login.notEnabledSubtitle')) }}
          </p>
        </div>

        <!-- 連不上後端：此時談密碼沒有意義，先讓使用者知道是連線問題 -->
        <template v-if="!reachable">
          <VAlert type="warning" variant="tonal" density="compact" class="mb-4" icon="tabler-alert-triangle">
            {{ t('page.login.offlineHint') }}
          </VAlert>
          <VBtn color="primary" size="large" block variant="tonal" @click="retryNow">
            <VIcon icon="tabler-refresh" size="18" class="me-1" />
            {{ t('page.login.retry') }}
          </VBtn>
        </template>

        <!-- 後端沒對外開放：這不是密碼問題，表單也填不了 -->
        <template v-else-if="!enabled">
          <VAlert type="warning" variant="tonal" density="compact" class="mb-4" icon="tabler-shield-off">
            {{ t('page.login.notEnabled') }}
          </VAlert>
          <VBtn color="primary" size="large" block variant="tonal" @click="retryNow">
            <VIcon icon="tabler-refresh" size="18" class="me-1" />
            {{ t('page.login.retry') }}
          </VBtn>
        </template>

        <template v-else>
          <VAlert v-if="!passwordSet" type="warning" variant="tonal" density="compact" class="mb-4" icon="tabler-alert-triangle">
            {{ t('page.login.noPassword') }}
          </VAlert>

          <VForm @submit.prevent="submit">
            <VTextField
              ref="passwordField"
              v-model="password"
              :label="t('page.login.password')"
              :type="showPassword ? 'text' : 'password'"
              :append-inner-icon="showPassword ? 'tabler-eye-off' : 'tabler-eye'"
              :disabled="submitting || !passwordSet"
              autocomplete="current-password"
              variant="outlined"
              @click:append-inner="showPassword = !showPassword"
            />

            <VAlert v-if="errorMsg" type="error" variant="tonal" density="compact" class="mt-3" icon="tabler-alert-circle">
              {{ errorMsg }}
            </VAlert>

            <VBtn type="submit" color="primary" size="large" block class="mt-5" :loading="submitting" :disabled="!password || !passwordSet">
              {{ t('page.login.submit') }}
            </VBtn>
          </VForm>

          <p class="text-caption text-medium-emphasis text-center mt-6 mb-0">
            {{ t('page.login.lanHint') }}
          </p>
        </template>
      </VCardText>
    </VCard>
  </div>
</template>

<style scoped>
.login-wrap {
  display: flex;
  align-items: center;
  justify-content: center;
  min-block-size: 100vh;
  padding: 16px;
}

.login-card {
  inline-size: 100%;
  max-inline-size: 420px;
}
</style>
