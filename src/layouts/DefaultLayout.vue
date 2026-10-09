<script setup>
import { onBeforeUnmount, onMounted } from 'vue'
import { RouterLink } from 'vue-router'
import { VerticalNavLayout, layoutConfig } from '@layouts'
import { useLayoutConfigStore } from '@layouts/stores/config'
import { useSkins } from '@core/composable/useSkins'
import AppNavbar from '@/components/AppNavbar.vue'
import AppLogo from '@/components/AppLogo.vue'
import { navItems } from '@/config/navConfig'
import { useStatusStore } from '@/stores/status'
import { listen } from '@/api/events'
import { toast } from 'vue3-toastify'
import { useI18n } from 'vue-i18n'

const { layoutAttrs } = useSkins()
const configStore = useLayoutConfigStore()
const status = useStatusStore()
const visibleNavItems = computed(() => navItems.filter(i => !i.printing || status.print.enabled))
const { t } = useI18n()

const STICKY_ACTIONS = new Set(['task_panic', 'crash_stop'])
// 頂端警示按掉後，同一批停止的工作不再顯示；重新整理或又有別的工作停止時再出現
const dismissedStopped = ref('')
const stoppedKey = computed(() => status.stoppedTasks.map(s => `${s.name}@${s.at}`).join('|'))
const showStopped = computed(() => !!stoppedKey.value && stoppedKey.value !== dismissedStopped.value)

let unlistenAlert = null
let unlistenMsg = null
let unlistenParcel = null

onMounted(() => {
  status.start()
  // 印表機告警與錯誤級系統訊息：任何頁面都要跳出來
  unlistenAlert = listen('printer-alert', ({ payload }) => {
    toast(`${payload.message}（${payload.port}）`, { type: 'error', autoClose: 8000 })
  })
  unlistenMsg = listen('system-message', ({ payload }) => {
    // 背景工作崩潰（含因此自動停皮帶）：功能要到程式重開才恢復，提示留著直到有人按掉，並更新頂端警示
    if (STICKY_ACTIONS.has(payload.action)) {
      toast(payload.message, { type: 'error', autoClose: false })
      status.refresh()
      return
    }
    if (payload.level === 'error') toast(payload.message, { type: 'error', autoClose: 6000 })
    // 卡件解除但人員停過線而不自動啟動：現場要知道皮帶為什麼沒動
    else if (payload.action === 'auto_start_skipped') toast(payload.message, { type: 'warning', autoClose: 12000 })
  })
  // 包裹層級的現場提示：同一件反覆走異常口、已完成卻又進線、同模組反覆卡件——要停留久一點讓人看到條碼
  unlistenParcel = listen('parcel-alert', ({ payload }) => {
    toast(payload.message, { type: 'warning', autoClose: 12000 })
  })
})

onBeforeUnmount(() => {
  status.stop()
  unlistenAlert?.()
  unlistenMsg?.()
  unlistenParcel?.()
})
</script>

<template>
  <VerticalNavLayout :home-url="'/'" :nav-items="visibleNavItems" :vertical-nav-attrs="layoutAttrs.verticalNavAttrs">
    <template #vertical-nav-header>
      <RouterLink to="/" class="app-logo app-title-wrapper">
        <AppLogo />
        <h1 class="app-logo-title leading-normal">{{ t('app.name') }}</h1>
        <span v-if="status.version" class="app-version-badge text-body-small font-weight-medium">v{{ status.version }}</span>
      </RouterLink>
      <div class="nav-collapse-btn">
        <Component :is="layoutConfig.app.iconRenderer || 'div'" v-if="configStore.isVerticalNavCollapsed" v-bind="layoutConfig.icons.verticalNavUnPinned" @click="configStore.isVerticalNavCollapsed = false" />
        <Component :is="layoutConfig.app.iconRenderer || 'div'" v-else v-bind="layoutConfig.icons.verticalNavPinned" @click="configStore.isVerticalNavCollapsed = true" />
      </div>
    </template>

    <template #navbar="{ toggleVerticalOverlayNavActive }">
      <AppNavbar :toggle-vertical-overlay-nav-active="toggleVerticalOverlayNavActive" />
    </template>

    <VAlert v-if="showStopped" type="error" variant="tonal" icon="tabler-alert-octagon" closable class="mb-4" @click:close="dismissedStopped = stoppedKey">
      <div v-for="s in status.stoppedTasks" :key="s.name + s.at">{{ t('app.stoppedTask', { name: s.name, at: s.at }) }}</div>
    </VAlert>

    <slot />

  </VerticalNavLayout>
</template>

<style lang="scss" scoped>
.app-logo { display: flex; align-items: center; column-gap: 0.75rem; text-decoration: none; color: inherit; margin-inline-end: auto; }
.app-logo-title { font-size: 1.375rem; font-weight: 700; letter-spacing: 0.25px; line-height: 1.5rem; }
.app-version-badge { color: #fff; opacity: 0.85; margin-inline-start: 0.25rem; align-self: flex-end; padding-block-end: 2px; }
.nav-collapse-btn {
  cursor: pointer; font-size: 1.25rem; flex-shrink: 0;
  color: rgba(var(--v-theme-on-surface), var(--v-medium-emphasis-opacity));
  &:hover { color: rgba(var(--v-theme-on-surface), var(--v-high-emphasis-opacity)); }
}
</style>

<style lang="scss">
@use "@layouts/styles/default-layout";
.layout-vertical-nav .nav-collapse-btn { display: flex; align-items: center; cursor: pointer; font-size: 1.25rem; flex-shrink: 0; > * { display: inline-flex !important; } }
</style>
