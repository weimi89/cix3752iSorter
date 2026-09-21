<script setup>
import { useI18n } from 'vue-i18n'
import { api, parcelImageUrl } from '@/api/http'
import { listen } from '@/api/events'
import { fmtTimeMs, sourceMeta } from '@/composables/useFormat'
import AppHeader from '@/components/AppHeader.vue'
import PageActions from '@/components/PageActions.vue'
import ParcelDetailDialog from '@/components/ParcelDetailDialog.vue'
import ProtectedImg from '@/components/ProtectedImg.vue'
import { toast } from 'vue3-toastify'

const { t } = useI18n()
const tab = ref('pending')
const list = ref([])
const pending = ref(0)
const serverNow = ref(Date.now())
const loading = ref(false)
const errorMsg = ref('')
const busyId = ref(null)
const detailId = ref(null)
const detailOpen = ref(false)

const load = async () => {
  loading.value = true
  errorMsg.value = ''
  try {
    const d = await api.abnormalList({ state: tab.value, limit: 300 })
    list.value = d.list
    pending.value = d.pending
    serverNow.value = d.now_ms
  } catch (e) { errorMsg.value = e.message } finally { loading.value = false }
}
watch(tab, load)

// 等待時間用伺服器時鐘算（別台電腦開網頁時兩邊時鐘會差）
const now = ref(Date.now())
let clock = null
const ageMs = row => Math.max(0, serverNow.value + (now.value - loadedAt) - (row.ended_ms || 0))
let loadedAt = Date.now()
watch(serverNow, () => { loadedAt = Date.now() })
const fmtAge = ms => {
  const m = Math.floor(ms / 60000)
  if (m < 1) return t('page.abnormal.justNow')
  if (m < 60) return t('page.abnormal.minutes', { n: m })
  return t('page.abnormal.hours', { h: Math.floor(m / 60), m: m % 60 })
}
// 待處理超過 10 分鐘標紅：這件已經在異常口躺很久沒人理
const OVERDUE_MS = 10 * 60000

const reasonText = r => {
  const code = r.chute_reason || (r.chute_source === 'noread' ? 'NOREAD' : r.chute_source === 'timeout' ? 'TIMEOUT' : '')
  if (!code) return '—'
  const key = `page.stats.reason.${code}`
  return t(key) === key ? code : t(key)
}
// 讀碼失敗、逾時、回覆太晚：再投一次就好；其他（關轉、查無…）要下架
const suggestRefeed = r => ['NOREAD', 'TIMEOUT', 'LATE', 'MW_UNREACHABLE'].includes(r.chute_reason || (r.chute_source === 'noread' ? 'NOREAD' : ''))

const handle = async (row, state) => {
  busyId.value = row.id
  try {
    await api.abnormalHandle(row.id, state)
    toast(t(state === 'removed' ? 'page.abnormal.removedDone' : 'page.abnormal.refedDone', { barcode: row.barcode }), { type: 'success', autoClose: 3000 })
    await load()
  } catch (e) { toast(e.message, { type: 'error' }) } finally { busyId.value = null }
}
const reopen = async row => {
  busyId.value = row.id
  try { await api.abnormalReopen(row.id); await load() } catch (e) { toast(e.message, { type: 'error' }) } finally { busyId.value = null }
}
const open = id => { detailId.value = id; detailOpen.value = true }

const actions = computed(() => [
  { key: 'reload', label: t('common.reload'), icon: 'tabler-refresh', loading: loading.value, onClick: load },
])

let unlisten = []
onMounted(() => {
  load()
  clock = setInterval(() => { now.value = Date.now() }, 15000)
  let timer = null
  const soon = () => { clearTimeout(timer); timer = setTimeout(load, 1200) }
  unlisten = [
    listen('abnormal-updated', soon),
    // 新的異常件落格（終態）也要出現在清單上
    listen('parcel-updated', ({ payload }) => { if (payload.ended_ms) soon() }),
    // 照片通常比落格晚一兩秒到，到了再刷一次縮圖才會出現
    listen('parcel-image', soon),
  ]
})
onBeforeUnmount(() => { unlisten.forEach(u => u()); clearInterval(clock) })
</script>

<template>
  <div>
    <AppHeader :title="$t('page.abnormal.title')" :subtitle="$t('page.abnormal.subtitle')" :subtitle-short="$t('page.abnormal.subtitleShort')" icon="tabler-alert-triangle">
      <template #actions><PageActions :items="actions" /></template>
    </AppHeader>

    <VAlert v-if="errorMsg" type="error" variant="tonal" class="mb-3">{{ errorMsg }}</VAlert>

    <VCard>
      <VTabs v-model="tab" color="primary" class="px-2">
        <VTab value="pending">{{ $t('page.abnormal.tabPending') }}<VChip v-if="pending" size="x-small" color="error" class="ms-2" label>{{ pending }}</VChip></VTab>
        <VTab value="handled">{{ $t('page.abnormal.tabHandled') }}</VTab>
        <VTab value="all">{{ $t('common.all') }}</VTab>
      </VTabs>
      <VDivider />
      <VTable hover class="table-cards abnormal-table">
        <thead><tr>
          <th class="text-center" style="width: 120px;">{{ $t('page.abnormal.landedAt') }}</th>
          <th class="text-center">{{ $t('parcel.barcode') }}</th>
          <th class="text-center" style="width: 130px;">{{ $t('page.abnormal.reason') }}</th>
          <th class="text-center" style="width: 110px;">{{ $t('page.abnormal.waiting') }}</th>
          <th class="text-center" style="width: 150px;">{{ $t('page.abnormal.state') }}</th>
          <th class="text-center" style="width: 220px;">{{ $t('common.actions') }}</th>
        </tr></thead>
        <tbody>
          <tr v-if="!list.length"><td colspan="6"><div class="py-3 d-flex align-center justify-center"><VIcon icon="tabler-circle-check" size="20" class="me-1 text-success" /><span class="text-md">{{ tab === 'pending' ? $t('page.abnormal.nonePending') : $t('common.noResults') }}</span></div></td></tr>
          <tr v-for="r in list" :key="r.id" :class="{ 'abnormal-overdue': !r.state && ageMs(r) >= OVERDUE_MS }">
            <td :data-label="$t('page.abnormal.landedAt')" class="text-center text-no-wrap">
              <div>{{ fmtTimeMs(r.ended_ms).slice(0, 8) }}</div>
              <!-- 讀碼站照片縮圖放在時間下面，不另開一欄：欄位已經很擠，桌面與平板都塞不下第七欄 -->
              <ProtectedImg v-if="r.image_id" :src="parcelImageUrl(r.image_id)" :alt="r.barcode" class="abnormal-thumb mt-1" @click="open(r.id)" />
            </td>
            <td :data-label="$t('parcel.barcode')" class="text-center"><span class="selectable font-weight-medium cursor-pointer" @click="open(r.id)">{{ r.barcode }}</span></td>
            <td :data-label="$t('page.abnormal.reason')" class="text-center"><VChip size="x-small" :color="sourceMeta(r.chute_source).color" variant="tonal" label>{{ reasonText(r) }}</VChip></td>
            <td :data-label="$t('page.abnormal.waiting')" class="text-center text-no-wrap" :class="{ 'text-error font-weight-bold': !r.state && ageMs(r) >= OVERDUE_MS }">{{ r.state ? '—' : fmtAge(ageMs(r)) }}</td>
            <td :data-label="$t('page.abnormal.state')" class="text-center">
              <VChip v-if="!r.state" size="x-small" color="warning" label>{{ $t('page.abnormal.pending') }}</VChip>
              <template v-else>
                <VChip size="x-small" :color="r.state === 'removed' ? 'secondary' : 'success'" label>{{ $t(r.state === 'removed' ? 'page.abnormal.removed' : 'page.abnormal.refed') }}</VChip>
                <div class="text-body-small text-medium-emphasis mt-1">{{ fmtTimeMs(r.handled_ms).slice(0, 8) }} · {{ $t(`page.abnormal.by.${r.handled_by}`) }}</div>
              </template>
            </td>
            <td :data-label="$t('common.actions')" class="text-center">
              <template v-if="!r.state">
                <VBtn size="small" :variant="suggestRefeed(r) ? 'flat' : 'tonal'" color="success" :loading="busyId === r.id" class="me-1" @click="handle(r, 'refed')"><VIcon icon="tabler-refresh" size="16" class="me-1" />{{ $t('page.abnormal.markRefed') }}</VBtn>
                <VBtn size="small" :variant="suggestRefeed(r) ? 'tonal' : 'flat'" color="secondary" :loading="busyId === r.id" @click="handle(r, 'removed')"><VIcon icon="tabler-trash" size="16" class="me-1" />{{ $t('page.abnormal.markRemoved') }}</VBtn>
              </template>
              <VBtn v-else size="small" variant="text" :loading="busyId === r.id" @click="reopen(r)">{{ $t('page.abnormal.reopen') }}</VBtn>
            </td>
          </tr>
        </tbody>
      </VTable>
    </VCard>

    <ParcelDetailDialog v-model="detailOpen" :parcel-id="detailId" />
  </div>
</template>

<style scoped>
.abnormal-overdue { background: rgba(var(--v-theme-error), 0.06); }
.abnormal-thumb { inline-size: 64px; block-size: 48px; object-fit: cover; border-radius: 4px; cursor: pointer; vertical-align: middle; }
@media (min-width: 640px) {
  .abnormal-table :deep(table) { table-layout: fixed; inline-size: 100%; }
}
</style>
