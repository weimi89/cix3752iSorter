<script setup>
/** 印表機：各格口對應的 USB 埠位與在線狀態、測試頁；目前接著的 USB 印表機清單 */
import { useI18n } from 'vue-i18n'
import { api } from '@/api/http'
import { listen } from '@/api/events'
import AppHeader from '@/components/AppHeader.vue'
import PageActions from '@/components/PageActions.vue'
import { toast } from 'vue3-toastify'

const { t } = useI18n()
const data = ref({ attached: [], configured: [] })
const loading = ref(false)
const errorMsg = ref('')
const testing = ref('')

const load = async () => {
  loading.value = true
  errorMsg.value = ''
  try { data.value = await api.printers() } catch (e) { errorMsg.value = e.message } finally { loading.value = false }
}
const test = async port => {
  testing.value = port
  try { await api.printerTest(port); toast(t('page.printers.testSent'), { type: 'success' }) } catch (e) { toast(e.message, { type: 'error' }) } finally { testing.value = '' }
}
const actions = computed(() => [{ key: 'reload', label: t('common.reload'), icon: 'tabler-refresh', loading: loading.value, onClick: load }])
const offlineCount = computed(() => data.value.configured.filter(c => !c.online).length)
// 後端照偵測順序回，孔位打散難對照；依 bus-port 數字排
const attachedSorted = computed(() => [...data.value.attached].sort((a, b) => a.port.localeCompare(b.port, undefined, { numeric: true })))

let unlisten = null
onMounted(() => { load(); unlisten = listen('printer-alert', load) })
onBeforeUnmount(() => unlisten?.())
</script>

<template>
  <div>
    <AppHeader :title="$t('page.printers.title')" :subtitle="$t('page.printers.subtitle')" :subtitle-short="$t('page.printers.subtitleShort')" icon="tabler-printer" sticky>
      <template #actions><PageActions :items="actions" /></template>
    </AppHeader>
    <VAlert v-if="errorMsg" type="error" variant="tonal" class="mb-3">{{ errorMsg }}</VAlert>
    <VAlert v-else-if="!data.attached.length" type="warning" variant="tonal" class="mb-3" icon="tabler-usb">{{ $t('page.printers.noneAttached') }}</VAlert>
    <VAlert v-else-if="offlineCount" type="error" variant="tonal" class="mb-3" icon="tabler-plug-connected-x">{{ $t('page.printers.offlineAlert', { n: offlineCount }) }}</VAlert>

    <VRow density="compact">
      <VCol v-for="c in data.configured" :key="c.chute" cols="12" sm="6" lg="3" class="py-1">
        <VCard class="card-shadow h-100">
          <VCardTitle class="d-flex align-center justify-space-between px-4 py-3">
            <div class="d-flex align-center"><VIcon icon="tabler-printer" size="22" class="me-2" :color="c.online ? 'success' : 'error'" />{{ c.chute }}</div>
            <VChip size="x-small" :color="c.online ? 'success' : 'error'" label variant="tonal">{{ c.online ? $t('page.printers.online') : $t('page.printers.offline') }}</VChip>
          </VCardTitle>
          <VDivider />
          <VCardText class="pt-3 d-flex align-center">
            <div class="text-body-small">
              <div class="text-medium-emphasis">{{ $t('page.printers.port') }}</div>
              <div class="font-weight-medium">{{ c.port }}<template v-if="c.device"> · <code>{{ c.device.split('/').pop() }}</code></template></div>
            </div>
            <VSpacer />
            <VBtn size="small" variant="tonal" color="primary" :disabled="!c.online" :loading="testing === c.port" @click="test(c.port)"><VIcon icon="tabler-file-text" size="16" class="me-1" />{{ $t('page.printers.test') }}</VBtn>
          </VCardText>
        </VCard>
      </VCol>
      <VCol v-if="!data.configured.length" cols="12">
        <VCard class="card-shadow"><VCardText class="text-center text-medium-emphasis py-6">{{ $t('page.printers.noneConfigured') }}</VCardText></VCard>
      </VCol>
    </VRow>

    <VCard class="card-shadow mt-4">
      <VCardTitle class="d-flex align-center justify-space-between px-4 py-3">
        <div class="d-flex align-center"><VIcon icon="tabler-usb" size="22" class="me-2" />{{ $t('page.printers.attached') }}</div>
        <span class="text-body-small text-medium-emphasis">{{ data.attached.length }} {{ $t('page.printers.unit') }}</span>
      </VCardTitle>
      <VDivider />
      <VList density="compact" class="py-1">
        <VListItem v-for="a in attachedSorted" :key="a.port">
          <template #prepend><VIcon icon="tabler-usb" size="18" class="me-2" /></template>
          <VListItemTitle class="text-body-medium">{{ a.port }}</VListItemTitle>
          <VListItemSubtitle class="text-body-small"><code>{{ a.device }}</code></VListItemSubtitle>
          <template #append><VBtn size="small" variant="text" color="primary" :loading="testing === a.port" @click="test(a.port)">{{ $t('page.printers.test') }}</VBtn></template>
        </VListItem>
        <VListItem v-if="!data.attached.length"><VListItemTitle class="text-medium-emphasis">{{ $t('common.noData') }}</VListItemTitle></VListItem>
      </VList>
    </VCard>
  </div>
</template>
