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
const statusColor = { pending: 'secondary', printing: 'info', done: 'success', failed: 'error' }
const STATUSES = computed(() => [{ value: null, title: t('common.all') }, ...['pending', 'printing', 'done', 'failed'].map(s => ({ value: s, title: t(`print.s.${s}`) }))])

const load = async () => {
  loading.value = true
  errorMsg.value = ''
  try {
    const d = await api.printJobs({ status: status.value, q: q.value, barcode: barcode.value, start: startDate.value ? `${startDate.value} 00:00:00` : '', end: endDate.value ? `${endDate.value} 23:59:59.999` : '', chute: chute.value, page: page.value, limit: pageSize.value })
    total.value = d.total
    list.value = d.list
  } catch (e) { errorMsg.value = e.message } finally { loading.value = false }
}
const search = () => { page.value = 1; load() }
const resetSearch = () => { status.value = null; q.value = ''; barcode.value = ''; startDate.value = ''; endDate.value = ''; chute.value = ''; search() }
const retry = async id => {
  try { await api.printJobRetry(id); toast(t('common.done'), { type: 'success' }); load() } catch (e) { toast(e.message, { type: 'error' }) }
}
const actions = computed(() => [{ key: 'reload', label: t('common.reload'), icon: 'tabler-refresh', loading: loading.value, onClick: load }])

// 面單預覽：只有還沒印掉的任務有點陣檔可看
const preview = ref({ open: false, job: null, src: '', error: '' })
const openPreview = j => { preview.value = { open: true, job: j, src: api.printJobPreviewUrl(j.id), error: '' } }
const previewFailed = () => { preview.value.error = t('print.previewHint') }

watch(pageSize, () => { page.value = 1; load() })
watch(page, load)
let unlisten = null
let timer = null
onMounted(async () => {
  try { chutes.value = (await api.chutes()).map(c => c.code) } catch {}
  load()
  unlisten = listen('print-job', () => { if (page.value !== 1) return; clearTimeout(timer); timer = setTimeout(load, 800) })
})
onBeforeUnmount(() => { unlisten?.(); clearTimeout(timer) })
</script>

<template>
  <div>
    <AppHeader :title="$t('page.printJobs.title')" :subtitle="$t('page.printJobs.subtitle')" :subtitle-short="$t('page.printJobs.subtitleShort')" icon="tabler-printer">
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
            <VCol cols="12" sm="6" lg="4" class="px-2 py-1"><div class="search-field"><label>{{ $t('print.status') }}</label><VSelect v-model="status" :items="STATUSES" density="compact" hide-details variant="outlined" /></div></VCol>
            <VCol cols="12" sm="6" lg="4" class="px-2 py-1"><div class="search-field"><label>{{ $t('common.keyword') }}</label><VTextField v-model="q" :placeholder="$t('page.printJobs.searchHint')" density="compact" hide-details variant="outlined" clearable @keyup.enter="search" /></div></VCol>
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
          <th class="text-center" style="width: 170px;">{{ $t('print.createdAt') }}</th><th class="text-center">{{ $t('parcel.barcode') }}</th><th class="text-center" style="width: 80px;">{{ $t('parcel.chute') }}</th><th class="text-center" style="width: 100px;">{{ $t('print.printer') }}</th><th class="text-center">{{ $t('print.profile') }}</th><th class="text-center" style="width: 100px;">{{ $t('print.status') }}</th><th class="text-center" style="width: 70px;">{{ $t('print.attempts') }}</th><th class="text-center">{{ $t('print.error') }}</th><th class="text-center" style="width: 90px;">{{ $t('common.actions') }}</th>
        </tr></thead>
        <tbody>
          <tr v-if="!list.length"><td colspan="9"><div class="py-2 d-flex align-center justify-center"><VIcon icon="tabler-alert-circle" size="20" class="me-1" /><span class="text-md">{{ $t('common.noResults') }}</span></div></td></tr>
          <tr v-for="j in list" :key="j.id">
            <td :data-label="$t('print.createdAt')" class="text-center text-no-wrap">{{ fmtMs(j.created_ms) }}</td>
            <td :data-label="$t('parcel.barcode')" class="text-center selectable">{{ j.barcode }}</td>
            <td :data-label="$t('parcel.chute')" class="text-center">{{ j.chute_code }}</td>
            <td :data-label="$t('print.printer')" class="text-center">{{ j.printer_port }}</td>
            <td :data-label="$t('print.profile')" class="text-center"><code>{{ j.profile }}</code></td>
            <td :data-label="$t('print.status')" class="text-center"><VChip size="x-small" :color="statusColor[j.status]" label>{{ $t(`print.s.${j.status}`) }}</VChip></td>
            <td :data-label="$t('print.attempts')" class="text-center">{{ j.attempts }}</td>
            <td :data-label="$t('print.error')" class="text-center text-error">{{ j.last_error }}</td>
            <td :data-label="$t('common.actions')" class="text-center text-no-wrap">
              <VBtn v-if="j.status !== 'done'" size="small" variant="text" class="me-1" @click="openPreview(j)"><VIcon icon="tabler-eye" size="16" class="me-1" />{{ $t('print.preview') }}</VBtn>
              <VBtn v-if="j.status !== 'done'" size="small" variant="tonal" @click="retry(j.id)">{{ $t('common.retry') }}</VBtn>
            </td>
          </tr>
        </tbody>
      </VTable>
      <VDivider />
      <TablePagination v-model:page="page" v-model:per-page="pageSize" :total="total" />
    </VCard>

    <VDialog v-model="preview.open" max-width="520">
      <VCard>
        <VCardTitle class="d-flex align-center">
          <span>{{ $t('print.previewTitle') }}</span>
          <span v-if="preview.job" class="text-body-medium text-medium-emphasis ms-3">{{ preview.job.chute_code }} · {{ preview.job.barcode }}</span>
          <VSpacer />
          <VBtn icon variant="text" @click="preview.open = false"><VIcon icon="tabler-x" /></VBtn>
        </VCardTitle>
        <VCardText class="text-center">
          <VAlert v-if="preview.error" type="warning" variant="tonal" density="compact">{{ preview.error }}</VAlert>
          <img v-else :src="preview.src" :alt="$t('print.previewTitle')" class="label-preview" @error="previewFailed">
          <div class="text-body-small text-medium-emphasis mt-2">{{ $t('print.previewHint') }}</div>
        </VCardText>
      </VCard>
    </VDialog>
  </div>
</template>

<style scoped>
.label-preview { max-width: 100%; max-height: 70vh; border: 1px solid rgba(var(--v-border-color), var(--v-border-opacity)); background: #fff; }
</style>
