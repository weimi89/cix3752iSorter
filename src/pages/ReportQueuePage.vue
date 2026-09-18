<script setup>
import { useI18n } from 'vue-i18n'
import { api } from '@/api/http'
import { listen } from '@/api/events'
import { fmtMs } from '@/composables/useFormat'
import AppHeader from '@/components/AppHeader.vue'
import PageActions from '@/components/PageActions.vue'
import TablePagination from '@/components/TablePagination.vue'
import MultiNoField from '@/components/MultiNoField.vue'
import AppDatePicker from '@/components/AppDatePicker.vue'
import { toast } from 'vue3-toastify'

const { t } = useI18n()
const status = ref(null)
const q = ref('')
const barcode = ref('')
const startDate = ref('')
const endDate = ref('')
const chute = ref('')
const chutes = ref([])
const chuteItems = computed(() => [{ value: '', title: t('common.all') }, ...chutes.value.map(c => ({ value: c, title: c }))])
const page = ref(1)
const pageSize = ref(25)
const total = ref(0)
const list = ref([])
const loading = ref(false)
const errorMsg = ref('')
const searchOpen = ref(0)
const statusColor = { pending: 'secondary', sending: 'info', success: 'success', failed: 'error' }
const STATUSES = computed(() => [{ value: null, title: t('common.all') }, ...['pending', 'sending', 'success', 'failed'].map(s => ({ value: s, title: t(`report.s.${s}`) }))])

const load = async () => {
  loading.value = true
  errorMsg.value = ''
  try {
    const d = await api.reportQueue({ status: status.value, q: q.value, barcode: barcode.value, start: startDate.value ? `${startDate.value} 00:00:00` : '', end: endDate.value ? `${endDate.value} 23:59:59.999` : '', chute: chute.value, page: page.value, limit: pageSize.value })
    total.value = d.total
    list.value = d.list
  } catch (e) { errorMsg.value = e.message } finally { loading.value = false }
}
const search = () => { page.value = 1; load() }
const resetSearch = () => { status.value = null; q.value = ''; barcode.value = ''; startDate.value = ''; endDate.value = ''; chute.value = ''; search() }
const retry = async id => {
  try { await api.reportRetry(id); toast(t('common.done'), { type: 'success' }); load() } catch (e) { toast(e.message, { type: 'error' }) }
}
const actions = computed(() => [{ key: 'reload', label: t('common.reload'), icon: 'tabler-refresh', loading: loading.value, onClick: load }])

watch(pageSize, () => { page.value = 1; load() })
watch(page, load)
let unlisten = null
let timer = null
onMounted(async () => {
  try { chutes.value = (await api.chutes()).map(c => c.code) } catch {}
  load()
  unlisten = listen('report-queue', () => { if (page.value !== 1) return; clearTimeout(timer); timer = setTimeout(load, 800) })
})
onBeforeUnmount(() => { unlisten?.(); clearTimeout(timer) })
</script>

<template>
  <div>
    <AppHeader :title="$t('page.reportQueue.title')" :subtitle="$t('page.reportQueue.subtitle')" :subtitle-short="$t('page.reportQueue.subtitleShort')" icon="tabler-cloud-upload">
      <template #actions><PageActions :items="actions" /></template>
    </AppHeader>
    <VAlert v-if="errorMsg" type="error" variant="tonal" class="mb-3">{{ errorMsg }}</VAlert>

    <VExpansionPanels v-model="searchOpen" class="mb-3 advanced-search">
      <VExpansionPanel>
        <VExpansionPanelTitle class="advanced-search__title">{{ $t('common.advancedSearch') }}</VExpansionPanelTitle>
        <VExpansionPanelText>
          <VRow no-gutters class="mx-n2">
            <VCol cols="12" sm="6" lg="3" class="px-2 py-1"><div class="search-field"><label>{{ $t('page.parcels.from') }}</label><AppDatePicker v-model="startDate" :max="endDate" /></div></VCol>
            <VCol cols="12" sm="6" lg="3" class="px-2 py-1"><div class="search-field"><label>{{ $t('page.parcels.to') }}</label><AppDatePicker v-model="endDate" :min="startDate" /></div></VCol>
            <VCol cols="12" lg="6" class="px-2 py-1"><div class="search-field"><label>{{ $t('parcel.barcode') }}</label><MultiNoField v-model="barcode" @search="search" /></div></VCol>
            <VCol cols="12" sm="6" lg="4" class="px-2 py-1"><div class="search-field"><label>{{ $t('parcel.chute') }}</label><VSelect v-model="chute" :items="chuteItems" density="compact" hide-details variant="outlined" /></div></VCol>
            <VCol cols="12" sm="6" lg="4" class="px-2 py-1"><div class="search-field"><label>{{ $t('report.status') }}</label><VSelect v-model="status" :items="STATUSES" density="compact" hide-details variant="outlined" /></div></VCol>
            <VCol cols="12" sm="6" lg="4" class="px-2 py-1"><div class="search-field"><label>{{ $t('parcel.responseId') }}</label><VTextField v-model="q" :placeholder="$t('page.reportQueue.searchHint')" density="compact" hide-details variant="outlined" clearable @keyup.enter="search" /></div></VCol>
          </VRow>
          <div class="d-flex justify-center ga-2 pt-4">
            <VBtn variant="outlined" color="secondary" @click="resetSearch"><VIcon icon="tabler-eraser" size="18" class="me-1" />{{ $t('common.reset') }}</VBtn>
            <VBtn variant="elevated" color="primary" :loading="loading" @click="search"><VIcon icon="tabler-database-search" size="18" class="me-1" />{{ $t('common.search') }}</VBtn>
          </div>
        </VExpansionPanelText>
      </VExpansionPanel>
    </VExpansionPanels>

    <VCard>
      <TablePagination v-model:page="page" v-model:per-page="pageSize" :total="total" header />
      <VDivider />
      <VTable hover class="table-cards">
        <thead><tr>
          <th class="text-center" style="width: 170px;">{{ $t('print.createdAt') }}</th><th class="text-center">{{ $t('parcel.barcode') }}</th><th class="text-center" style="width: 80px;">{{ $t('parcel.chute') }}</th><th class="text-center" style="width: 110px;">{{ $t('parcel.responseId') }}</th><th class="text-center" style="width: 100px;">{{ $t('report.status') }}</th><th class="text-center" style="width: 70px;">{{ $t('report.retries') }}</th><th class="text-center" style="width: 170px;">{{ $t('report.nextRetry') }}</th><th class="text-center">{{ $t('print.error') }}</th><th class="text-center" style="width: 90px;">{{ $t('common.actions') }}</th>
        </tr></thead>
        <tbody>
          <tr v-if="!list.length"><td colspan="9"><div class="py-2 d-flex align-center justify-center"><VIcon icon="tabler-alert-circle" size="20" class="me-1" /><span class="text-md">{{ $t('common.noResults') }}</span></div></td></tr>
          <tr v-for="r in list" :key="r.id">
            <td :data-label="$t('print.createdAt')" class="text-center text-no-wrap">{{ fmtMs(r.created_ms) }}</td>
            <td :data-label="$t('parcel.barcode')" class="text-center selectable">{{ r.barcode || '—' }}</td>
            <td :data-label="$t('parcel.chute')" class="text-center">{{ r.chute_code || '—' }}</td>
            <td :data-label="$t('parcel.responseId')" class="text-center">{{ r.response_id }}</td>
            <td :data-label="$t('report.status')" class="text-center"><VChip size="x-small" :color="statusColor[r.status]" label>{{ $t(`report.s.${r.status}`) }}</VChip></td>
            <td :data-label="$t('report.retries')" class="text-center">{{ r.retry_count }}</td>
            <td :data-label="$t('report.nextRetry')" class="text-center text-no-wrap">{{ r.status === 'pending' && r.next_retry_ms ? fmtMs(r.next_retry_ms) : '' }}</td>
            <td :data-label="$t('print.error')" class="text-center text-error">{{ r.last_error }}</td>
            <td :data-label="$t('common.actions')" class="text-center"><VBtn v-if="r.status === 'failed'" size="small" variant="tonal" @click="retry(r.id)">{{ $t('common.retry') }}</VBtn></td>
          </tr>
        </tbody>
      </VTable>
      <VDivider />
      <TablePagination v-model:page="page" v-model:per-page="pageSize" :total="total" />
    </VCard>
  </div>
</template>
