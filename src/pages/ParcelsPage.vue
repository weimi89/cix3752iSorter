<script setup>
import { useI18n } from 'vue-i18n'
import { api } from '@/api/http'
import { listen } from '@/api/events'
import { fmtDuration, statusMeta, sourceMeta, STATUS, SOURCE, fmtDate } from '@/composables/useFormat'
import AppHeader from '@/components/AppHeader.vue'
import PageActions from '@/components/PageActions.vue'
import TablePagination from '@/components/TablePagination.vue'
import ParcelDetailDialog from '@/components/ParcelDetailDialog.vue'
import AppDatePicker from '@/components/AppDatePicker.vue'
import MultiNoField from '@/components/MultiNoField.vue'
import { toast } from 'vue3-toastify'

const { t } = useI18n()
const today = fmtDate(new Date())
// 日期用日曆選；送後端時起日 00:00、迄日 23:59:59.999（整天）
const filters = reactive({ startDate: today, endDate: today, barcode: '', chute: '', status: null, source: null })
const startAt = () => filters.startDate ? `${filters.startDate} 00:00:00` : ''
const endAt = () => filters.endDate ? `${filters.endDate} 23:59:59.999` : ''
const page = ref(1)
const pageSize = ref(25)
const total = ref(0)
const list = ref([])
const loading = ref(false)
const errorMsg = ref('')
const detailId = ref(null)
const detailOpen = ref(false)
const chutes = ref([])
const searchOpen = ref(0)

const params = () => ({
  start: startAt(), end: endAt(), barcode: filters.barcode, chute: filters.chute,
  status: filters.status, source: filters.source, page: page.value, limit: pageSize.value,
})

const load = async () => {
  loading.value = true
  errorMsg.value = ''
  try {
    const d = await api.parcels(params())
    total.value = d.total
    list.value = d.list
  } catch (e) { errorMsg.value = e.message } finally { loading.value = false }
}
const search = () => { page.value = 1; load() }
const resetSearch = () => { Object.assign(filters, { startDate: today, endDate: today, barcode: '', chute: '', status: null, source: null }); search() }
const open = id => { detailId.value = id; detailOpen.value = true }
const exportNow = () => window.open(api.parcelsExportUrl(params()), '_blank')

const statusItems = computed(() => [{ value: null, title: t('common.all') }, ...Object.entries(STATUS).map(([v, m]) => ({ value: Number(v), title: t(m.key) }))])
const sourceItems = computed(() => [{ value: null, title: t('common.all') }, ...Object.entries(SOURCE).map(([v, m]) => ({ value: v, title: t(m.key) }))])
const chuteItems = computed(() => [{ value: '', title: t('common.all') }, ...chutes.value.map(c => ({ value: c, title: c }))])

const actions = computed(() => [
  { key: 'export', label: t('page.parcels.export'), icon: 'tabler-file-spreadsheet', variant: 'outlined', onClick: exportNow },
  { key: 'reload', label: t('common.reload'), icon: 'tabler-refresh', loading: loading.value, onClick: load },
])

watch(pageSize, () => { page.value = 1; load() })
watch(page, load)

let unlisten = null
onMounted(async () => {
  try { chutes.value = (await api.chutes()).map(c => c.code) } catch {}
  load()
  let timer = null
  unlisten = listen('parcel-updated', ({ payload }) => {
    if (page.value !== 1 || filters.barcode || !payload.ended_ms) return
    clearTimeout(timer)
    timer = setTimeout(load, 1500)
  })
})
onBeforeUnmount(() => unlisten?.())
</script>

<template>
  <div>
    <AppHeader :title="$t('page.parcels.title')" :subtitle="$t('page.parcels.subtitle')" :subtitle-short="$t('page.parcels.subtitleShort')" icon="tabler-packages">
      <template #actions><PageActions :items="actions" /></template>
    </AppHeader>

    <VAlert v-if="errorMsg" type="error" variant="tonal" class="mb-3">{{ errorMsg }}</VAlert>

    <VExpansionPanels v-model="searchOpen" class="mb-3 advanced-search">
      <VExpansionPanel>
        <VExpansionPanelTitle class="advanced-search__title">{{ $t('common.advancedSearch') }}</VExpansionPanelTitle>
        <VExpansionPanelText>
          <VRow no-gutters class="mx-n2">
            <VCol cols="12" sm="6" lg="3" class="px-2 py-1">
              <div class="search-field">
                <label>{{ $t('page.parcels.from') }}</label>
                <AppDatePicker v-model="filters.startDate" :max="filters.endDate" />
              </div>
            </VCol>
            <VCol cols="12" sm="6" lg="3" class="px-2 py-1">
              <div class="search-field">
                <label>{{ $t('page.parcels.to') }}</label>
                <AppDatePicker v-model="filters.endDate" :min="filters.startDate" />
              </div>
            </VCol>
            <VCol cols="12" lg="6" class="px-2 py-1"><div class="search-field"><label>{{ $t('parcel.barcode') }}</label><MultiNoField v-model="filters.barcode" @search="search" /></div></VCol>
            <VCol cols="12" sm="6" lg="4" class="px-2 py-1"><div class="search-field"><label>{{ $t('parcel.chute') }}</label><VSelect v-model="filters.chute" :items="chuteItems" density="compact" hide-details variant="outlined" /></div></VCol>
            <VCol cols="12" sm="6" lg="4" class="px-2 py-1"><div class="search-field"><label>{{ $t('parcel.status') }}</label><VSelect v-model="filters.status" :items="statusItems" density="compact" hide-details variant="outlined" /></div></VCol>
            <VCol cols="12" sm="6" lg="4" class="px-2 py-1"><div class="search-field"><label>{{ $t('parcel.source') }}</label><VSelect v-model="filters.source" :items="sourceItems" density="compact" hide-details variant="outlined" /></div></VCol>
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
          <th class="text-center" style="width: 190px;">{{ $t('parcel.startedAt') }}</th>
          <th class="text-center">{{ $t('parcel.barcode') }}</th>
          <th class="text-center" style="width: 90px;">{{ $t('parcel.chute') }}</th>
          <th class="text-center" style="width: 130px;">{{ $t('parcel.source') }}</th>
          <th class="text-center" style="width: 120px;">{{ $t('parcel.status') }}</th>
          <th class="text-center" style="width: 100px;">{{ $t('parcel.travel') }}</th>
          <th class="text-center" style="width: 80px;">{{ $t('parcel.cart') }}</th>
        </tr></thead>
        <tbody>
          <tr v-if="!list.length"><td colspan="7"><div class="py-2 d-flex align-center justify-center"><VIcon icon="tabler-alert-circle" size="20" class="me-1" /><span class="text-md">{{ $t('common.noResults') }}</span></div></td></tr>
          <tr v-for="p in list" :key="p.id" class="cursor-pointer" @click="open(p.id)">
            <td :data-label="$t('parcel.startedAt')" class="text-center text-no-wrap">{{ p.started_at }}</td>
            <td :data-label="$t('parcel.barcode')" class="text-center selectable font-weight-medium">{{ p.barcode }}</td>
            <td :data-label="$t('parcel.chute')" class="text-center">{{ p.chute_code || '—' }}</td>
            <td :data-label="$t('parcel.source')" class="text-center"><VChip size="x-small" :color="sourceMeta(p.chute_source).color" variant="tonal" label>{{ $t(sourceMeta(p.chute_source).key) }}</VChip></td>
            <td :data-label="$t('parcel.status')" class="text-center"><VChip size="x-small" :color="statusMeta(p.status).color" label>{{ $t(statusMeta(p.status).key) }}</VChip></td>
            <td :data-label="$t('parcel.travel')" class="text-center">{{ fmtDuration(p.travel_ms) }}</td>
            <td :data-label="$t('parcel.cart')" class="text-center">{{ p.cart ?? '' }}</td>
          </tr>
        </tbody>
      </VTable>
      <VDivider />
      <TablePagination v-model:page="page" v-model:per-page="pageSize" :total="total" />
    </VCard>

    <ParcelDetailDialog v-model="detailOpen" :parcel-id="detailId" />
  </div>
</template>
