<script setup>
import { useI18n } from 'vue-i18n'
import QRCode from 'qrcode'
import { toast } from 'vue3-toastify'
import { useStatusStore } from '@/stores/status'
import { useUpdater } from '@/composables/useUpdater'
import { api } from '@/api/http'
import { isTauriRuntime } from '@/api/runtime'
import { useWebAuth } from '@/composables/useWebAuth'
import UpdateDialog from '@/components/UpdateDialog.vue'

defineProps({
  toggleVerticalOverlayNavActive: { type: Function, required: false, default: () => {} },
})

const status = useStatusStore()
const { updateAvailable, updateInfo } = useUpdater()
const showUpdateDialog = ref(false)
const { t } = useI18n()

// 手機遙控連線資訊（網址 + QR）：同區網手機開 /control 啟停皮帶、看狀態
const remoteDialog = ref(false)
const remoteUrls = ref([])
const qrDataUrl = ref('')
const remoteLoading = ref(false)
const openRemoteDialog = async () => {
  remoteDialog.value = true
  remoteLoading.value = true
  qrDataUrl.value = ''
  try {
    const { ips, port } = await api.lanIps()
    remoteUrls.value = (ips || []).map(i => ({ name: i.name, addr: `${i.ip}:${port}/control`, url: `http://${i.ip}:${port}/control` }))
    if (remoteUrls.value[0]) qrDataUrl.value = await QRCode.toDataURL(remoteUrls.value[0].url, { width: 240, margin: 1 })
  } catch (e) {
    toast(e.message, { type: 'error' })
  } finally {
    remoteLoading.value = false
  }
}
// 瀏覽器版從區網 IP 開（非 https）時沒有 navigator.clipboard，退回選取文字讓人長按複製
const copyAddr = async addr => {
  if (!navigator.clipboard) { toast(t('navbar.copyManual'), { type: 'info' }); return }
  try { await navigator.clipboard.writeText(addr); toast(t('navbar.copied'), { type: 'success' }) } catch (e) { toast(e.message, { type: 'error' }) }
}

// 只有從外網登入的人有「登出」可按；桌面視窗與內網來源沒有登入態，按了也沒有意義
const { isLan, logout } = useWebAuth()
const canLogout = computed(() => !isTauriRuntime && !isLan.value)
const doLogout = async () => {
  await logout()
  window.location.hash = '#/login'
}

const deviceChips = computed(() => [
  { key: 'belt', label: 'device.belt', on: status.devices.belt.connected },
  { key: 'sorter', label: 'device.sorter', on: status.devices.sorter.connected },
  { key: 'camera', label: 'device.camera', on: status.devices.camera.connected },
])
</script>

<template>
  <div class="d-flex h-100 align-center">
    <VBtn icon variant="text" color="default" class="ms-n3 d-lg-none" @click="toggleVerticalOverlayNavActive(true)">
      <VIcon size="26" icon="tabler-menu-2" />
    </VBtn>

    <!-- 今日件數：任何頁面都看得到 -->
    <VChip class="flex-shrink-0" color="primary" variant="tonal" size="small">
      <VIcon icon="tabler-packages" size="16" start />
      <span class="font-weight-medium"><span class="d-none d-sm-inline">{{ $t('navbar.today') }} </span>{{ status.todayCount }}</span>
    </VChip>

    <!-- 裝置連線燈 -->
    <div class="d-none d-md-flex align-center ms-3 gap-2">
      <VChip v-for="d in deviceChips" :key="d.key" size="small" :color="d.on ? 'success' : 'error'" variant="tonal">
        <VIcon :icon="d.on ? 'tabler-plug-connected' : 'tabler-plug-connected-x'" size="14" start />
        {{ $t(d.label) }}
      </VChip>
    </div>

    <VSpacer />

    <VChip v-if="!status.sseConnected" size="small" color="warning" variant="tonal" class="me-2">
      <VIcon icon="tabler-wifi-off" size="14" start />{{ $t('navbar.sseDown') }}
    </VChip>
    <VBtn icon size="small" variant="text" color="default" @click="openRemoteDialog">
      <VIcon icon="tabler-device-mobile" size="22" />
      <VTooltip activator="parent" location="bottom">{{ $t('navbar.remote') }}</VTooltip>
    </VBtn>
    <VBtn v-if="updateAvailable" icon size="small" variant="text" color="warning" @click="showUpdateDialog = true">
      <VBadge dot color="warning"><VIcon icon="tabler-download" size="22" /></VBadge>
      <VTooltip activator="parent" location="bottom">{{ $t('updater.available', { version: updateInfo?.version }) }}</VTooltip>
    </VBtn>
    <VBtn v-if="canLogout" icon size="small" variant="text" color="default" @click="doLogout">
      <VIcon icon="tabler-logout" size="22" />
      <VTooltip activator="parent" location="bottom">{{ $t('navbar.logout') }}</VTooltip>
    </VBtn>
    <UpdateDialog v-model="showUpdateDialog" />

    <VDialog v-model="remoteDialog" max-width="440">
      <VCard>
        <VCardTitle class="d-flex align-center ga-2 pt-4 px-5">
          <VIcon icon="tabler-device-mobile" size="20" color="info" />
          {{ $t('navbar.remote') }}
        </VCardTitle>
        <VCardText class="px-5 pb-2">
          <div class="text-body-small text-medium-emphasis mb-4">{{ $t('navbar.remoteHint') }}</div>
          <div v-if="remoteLoading" class="d-flex justify-center py-8"><VProgressCircular indeterminate color="primary" /></div>
          <template v-else>
            <div v-if="qrDataUrl" class="d-flex justify-center mb-4">
              <img :src="qrDataUrl" alt="QR" style="border-radius: 8px; background: #fff; padding: 8px;">
            </div>
            <VAlert v-if="!remoteUrls.length" type="warning" variant="tonal" density="compact">{{ $t('navbar.remoteNoIp') }}</VAlert>
            <div v-for="u in remoteUrls" :key="u.addr" class="d-flex align-center ga-2 mb-2">
              <VChip size="small" variant="tonal" class="flex-shrink-0">{{ u.name }}</VChip>
              <a :href="u.url" target="_blank" class="text-body-medium flex-grow-1 text-truncate">{{ u.addr }}</a>
              <VBtn icon size="x-small" variant="text" @click="copyAddr(u.url)"><VIcon icon="tabler-copy" size="16" /></VBtn>
            </div>
          </template>
        </VCardText>
        <VCardActions class="px-5 pb-4"><VSpacer /><VBtn variant="text" @click="remoteDialog = false">{{ $t('common.close') }}</VBtn></VCardActions>
      </VCard>
    </VDialog>
  </div>
</template>
