<script setup>
import { useI18n } from 'vue-i18n'
import { api } from '@/api/http'
import { listen } from '@/api/events'
import AppHeader from '@/components/AppHeader.vue'
import PageActions from '@/components/PageActions.vue'
import TablePagination from '@/components/TablePagination.vue'
import MultiNoField from '@/components/MultiNoField.vue'
import AppDatePicker from '@/components/AppDatePicker.vue'
import { fmtDate } from '@/composables/useFormat'

const { t } = useI18n()
const today = fmtDate(new Date())
const level = ref(null)
const category = ref(null)
const q = ref('')
const startDate = ref(today)
const endDate = ref(today)
const page = ref(1)
const pageSize = ref(25)
const total = ref(0)
const list = ref([])
const loading = ref(false)
const errorMsg = ref('')
const searchOpen = ref(0)
const levelColor = { info: 'info', warn: 'warning', error: 'error' }
const categories = ['belt', 'sorter', 'camera', 'tracker', 'chute', 'middleware', 'printer', 'server']
const LEVELS = computed(() => [{ value: null, title: t('common.all') }, ...['info', 'warn', 'error'].map(s => ({ value: s, title: t(`log.level.${s}`) }))])
const CATEGORIES = computed(() => [{ value: null, title: t('common.all') }, ...categories.map(s => ({ value: s, title: t(`log.cat.${s}`) }))])

const load = async () => {
  loading.value = true
  errorMsg.value = ''
  try {
    const d = await api.logs({ level: level.value, category: category.value, q: q.value, start: startDate.value ? `${startDate.value} 00:00:00` : '', end: endDate.value ? `${endDate.value} 23:59:59.999` : '', page: page.value, limit: pageSize.value })
    total.value = d.total
    list.value = d.list
  } catch (e) { errorMsg.value = e.message } finally { loading.value = false }
}
const search = () => { page.value = 1; load() }
const resetSearch = () => { level.value = null; category.value = null; q.value = ''; startDate.value = today; endDate.value = today; search() }
const actions = computed(() => [{ key: 'reload', label: t('common.reload'), icon: 'tabler-refresh', loading: loading.value, onClick: load }])

watch(pageSize, () => { page.value = 1; load() })
watch(page, load)
let unlisten = null
onMounted(() => {
  load()
  // 第一頁且沒關鍵字時即時插入新訊息；其餘情況按重新整理才更新
  unlisten = listen('system-message', ({ payload }) => {
    if (page.value !== 1 || q.value) return
    if (level.value && payload.level !== level.value) return
    if (category.value && payload.category !== category.value) return
    list.value.unshift({ id: (list.value[0]?.id || 0) + 0.5, ...payload })
    if (list.value.length > pageSize.value) list.value.length = pageSize.value
    total.value += 1
  })
})
onBeforeUnmount(() => unlisten?.())
</script>

<template>
  <div>
    <AppHeader :title="$t('page.eventLog.title')" :subtitle="$t('page.eventLog.subtitle')" :subtitle-short="$t('page.eventLog.subtitleShort')" icon="tabler-bell-ringing">
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
            <VCol cols="12" lg="6" class="px-2 py-1"><div class="search-field"><label>{{ $t('common.keyword') }}</label><MultiNoField v-model="q" @search="search" /></div></VCol>
            <VCol cols="12" sm="6" lg="6" class="px-2 py-1"><div class="search-field"><label>{{ $t('log.levelLabel') }}</label><VSelect v-model="level" :items="LEVELS" density="compact" hide-details variant="outlined" /></div></VCol>
            <VCol cols="12" sm="6" lg="6" class="px-2 py-1"><div class="search-field"><label>{{ $t('log.category') }}</label><VSelect v-model="category" :items="CATEGORIES" density="compact" hide-details variant="outlined" /></div></VCol>
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
      <VTable hover class="table-cards event-table">
        <thead><tr>
          <th class="text-center" style="width: 190px;">{{ $t('log.time') }}</th><th class="text-center" style="width: 80px;">{{ $t('log.levelLabel') }}</th><th class="text-center" style="width: 110px;">{{ $t('log.category') }}</th><th class="text-center" style="width: 150px;">{{ $t('log.action') }}</th><th style="min-width: 200px;">{{ $t('log.message') }}</th>
        </tr></thead>
        <tbody>
          <tr v-if="!list.length"><td colspan="5"><div class="py-2 d-flex align-center justify-center"><VIcon icon="tabler-alert-circle" size="20" class="me-1" /><span class="text-md">{{ $t('common.noResults') }}</span></div></td></tr>
          <tr v-for="l in list" :key="l.id">
            <td :data-label="$t('log.time')" class="text-center">{{ l.created_at }}</td>
            <td :data-label="$t('log.levelLabel')" class="text-center"><span class="font-weight-medium" :class="`text-${levelColor[l.level] || 'grey'}`">{{ $t(`log.level.${l.level}`, l.level) }}</span></td>
            <td :data-label="$t('log.category')" class="text-center">{{ $t(`log.cat.${l.category}`, l.category) }}</td>
            <td :data-label="$t('log.action')" class="text-center"><code>{{ l.action }}</code></td>
            <td :data-label="$t('log.message')" class="selectable">{{ l.message }}</td>
          </tr>
        </tbody>
      </VTable>
      <VDivider />
      <TablePagination v-model:page="page" v-model:per-page="pageSize" :total="total" />
    </VCard>
  </div>
</template>

<style scoped lang="scss">
.event-table {
  th, td { white-space: nowrap; }
  td:last-child { white-space: normal; word-break: break-all; }
}
</style>
