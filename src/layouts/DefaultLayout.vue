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
const { t } = useI18n()

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
    if (payload.level === 'error') toast(payload.message, { type: 'error', autoClose: 6000 })
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
  <VerticalNavLayout :home-url="'/'" :nav-items="navItems" :vertical-nav-attrs="layoutAttrs.verticalNavAttrs">
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
