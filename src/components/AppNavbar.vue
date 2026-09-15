<script setup>
import { useStatusStore } from '@/stores/status'
import { useUpdater } from '@/composables/useUpdater'
import UpdateDialog from '@/components/UpdateDialog.vue'

defineProps({
  toggleVerticalOverlayNavActive: { type: Function, required: false, default: () => {} },
})

const status = useStatusStore()
const { updateAvailable, updateInfo } = useUpdater()
const showUpdateDialog = ref(false)

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
    <VBtn v-if="updateAvailable" icon size="small" variant="text" color="warning" @click="showUpdateDialog = true">
      <VBadge dot color="warning"><VIcon icon="tabler-download" size="22" /></VBadge>
      <VTooltip activator="parent" location="bottom">{{ $t('updater.available', { version: updateInfo?.version }) }}</VTooltip>
    </VBtn>
    <UpdateDialog v-model="showUpdateDialog" />
  </div>
</template>
