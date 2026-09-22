<script setup>
import { useI18n } from 'vue-i18n'
import { api, parcelImageUrl } from '@/api/http'
import { listen } from '@/api/events'
import { fmtDate, statusMeta } from '@/composables/useFormat'
import AppHeader from '@/components/AppHeader.vue'
import AppDatePicker from '@/components/AppDatePicker.vue'
import TablePagination from '@/components/TablePagination.vue'
import PageActions from '@/components/PageActions.vue'
import ParcelDetailDialog from '@/components/ParcelDetailDialog.vue'
import ProtectedImg from '@/components/ProtectedImg.vue'
import ParcelImageViewer from '@/components/ParcelImageViewer.vue'

const { t } = useI18n()

// 給主管事後翻的：某一天所有異常件的存證照，逐件標「為什麼」，統計哪種最多才知道現場要改什麼。
// 現場是多人快速作業，沒有人會停下來按「已處理」，所以這頁沒有處理流程，只有看與標記
const today = () => fmtDate(new Date())
const yesterday = () => { const d = new Date(); d.setDate(d.getDate() - 1); return fmtDate(d) }
// 預設看昨天：主管是事後翻整個班次的；班次報表點過來會帶日期與類別
const route = useRoute()
const day = ref(typeof route.query.day === 'string' && route.query.day ? route.query.day : yesterday())
const kind = ref(typeof route.query.kind === 'string' ? route.query.kind : '')
// 讀碼失敗的原因由系統從讀碼站回應判定（照片看不出來的沒人分得出，不做人工標記）
const cause = ref('')
const page = ref(1)
const pageSize = ref(24)
const items = ref([])
const total = ref(0)
const dayTotal = ref(0)
const byKind = ref({ sorter: 0, middleware: 0, noread: 0 })
const causeCounts = ref([])
const loading = ref(false)
const errorMsg = ref('')

const load = async () => {
  loading.value = true
  errorMsg.value = ''
  try {
    const r = await api.abnormalReview({ day: day.value, kind: kind.value, cause: cause.value, limit: pageSize.value, offset: (page.value - 1) * pageSize.value })
    items.value = r.items
    total.value = r.total
    dayTotal.value = r.day_total
    byKind.value = r.by_kind
    causeCounts.value = r.cause_counts
  } catch (e) { errorMsg.value = e.message } finally { loading.value = false }
}
const search = () => { page.value = 1; load() }
watch([day, kind, cause, pageSize], search)
watch(page, load)

const KINDS = [
  { value: '', color: 'primary', icon: 'tabler-list' },
  { value: 'noread', color: 'warning', icon: 'tabler-barcode-off' },
  { value: 'middleware', color: 'info', icon: 'tabler-cloud-x' },
  { value: 'sorter', color: 'error', icon: 'tabler-alert-circle' },
]
const kindCount = k => (k ? byKind.value[k] || 0 : dayTotal.value)
const kindColor = k => KINDS.find(x => x.value === k)?.color || 'secondary'

const CAUSE_COLOR = { no_code: 'warning', neighbor: 'error', bad_code: 'secondary', no_frame: 'info' }
const causeColor = code => CAUSE_COLOR[code] || 'secondary'
const causeTotal = computed(() => causeCounts.value.reduce((a, b) => a + b.count, 0))
const causePct = c => (causeTotal.value ? Math.round((c.count / causeTotal.value) * 100) : 0)
// 選了原因等於只看讀碼失敗
watch(cause, v => { if (v && kind.value !== 'noread') kind.value = 'noread' })
watch(kind, v => { if (v !== 'noread') cause.value = '' })

// 系統記的原因：讀碼失敗／仲介機回傳看原因代碼，分揀機異常看最終狀態
const systemReason = it => {
  if (it.kind === 'sorter') return { text: t(statusMeta(it.status).key), color: statusMeta(it.status).color }
  const code = it.chute_reason || (it.chute_source === 'noread' ? 'NOREAD' : it.chute_source === 'timeout' ? 'TIMEOUT' : '')
  if (!code) return { text: '—', color: 'secondary' }
  const key = `page.stats.reason.${code}`
  return { text: t(key) === key ? code : t(key), color: kindColor(it.kind) }
}

const viewerImage = ref(null)
const viewerOpen = ref(false)
const openImage = it => { if (it.image_id) { viewerImage.value = { id: it.image_id, has_orig: it.has_orig }; viewerOpen.value = true } }
const detailId = ref(null)
const detailOpen = ref(false)
const openDetail = it => { detailId.value = it.id; detailOpen.value = true }

const actions = computed(() => [
  { key: 'reload', label: t('common.reload'), icon: 'tabler-refresh', loading: loading.value, onClick: load },
])

let unlisten = []
onMounted(() => {
  load()
  // 看今天時，新的異常件（終態）與照片到了就補上；翻舊日子不動
  let timer = null
  const soon = () => { if (day.value === today() && page.value === 1) { clearTimeout(timer); timer = setTimeout(load, 1500) } }
  unlisten = [
    listen('parcel-updated', ({ payload }) => { if (payload.ended_ms) soon() }),
    listen('parcel-image', soon),
  ]
})
onBeforeUnmount(() => unlisten.forEach(u => u()))
</script>

<template>
  <div>
    <AppHeader :title="$t('page.abnormal.title')" :subtitle="$t('page.abnormal.subtitle')" :subtitle-short="$t('page.abnormal.subtitleShort')" icon="tabler-camera-search">
      <template #actions><PageActions :items="actions" /></template>
    </AppHeader>

    <VAlert v-if="errorMsg" type="error" variant="tonal" class="mb-3">{{ errorMsg }}</VAlert>

    <!-- 篩選 + 當天統計 -->
    <VCard class="mb-3 card-shadow">
      <VCardText>
        <div class="d-flex flex-wrap align-center gap-x-4 gap-y-3">
          <div style="inline-size: 180px;"><AppDatePicker v-model="day" :label="$t('page.abnormal.day')" :max="today()" /></div>
          <VSpacer />
          <div class="text-center">
            <div class="text-body-small text-medium-emphasis">{{ $t('page.abnormal.dayTotal') }}</div>
            <div class="text-headline-small font-weight-bold text-error">{{ dayTotal }}</div>
          </div>
        </div>
        <!-- 類別：讀碼失敗／仲介機回傳／分揀機異常，數字是整天的 -->
        <div class="d-flex flex-wrap gap-2 mt-3">
          <VChip v-for="k in KINDS" :key="k.value" :color="k.color" :variant="kind === k.value ? 'flat' : 'tonal'" size="small" label @click="kind = k.value">
            <VIcon :icon="k.icon" size="14" start />{{ k.value ? $t(`page.abnormal.kind.${k.value}`) : $t('common.all') }} {{ kindCount(k.value) }}
          </VChip>
        </div>
        <!-- 讀碼失敗的原因（系統從讀碼站回應判定）：點一個只看那個原因，再點一次回全部 -->
        <div class="d-flex flex-wrap align-center gap-2 mt-2">
          <span class="text-body-small text-medium-emphasis">{{ $t('page.abnormal.causes') }}</span>
          <VChip v-for="c in causeCounts" :key="c.cause" :color="causeColor(c.cause)" :variant="cause === c.cause ? 'flat' : 'tonal'" size="small" label @click="cause = cause === c.cause ? '' : c.cause">
            {{ $t(`page.abnormal.cause.${c.cause}`) }} {{ c.count }}<span v-if="c.count" class="ms-1 text-medium-emphasis">({{ causePct(c) }}%)</span>
          </VChip>
        </div>
        <div class="text-body-small text-disabled mt-2">{{ $t('page.abnormal.hint') }}</div>
      </VCardText>
    </VCard>

    <VCard class="card-shadow">
      <TablePagination v-model:page="page" v-model:per-page="pageSize" :total="total" :page-sizes="[12, 24, 48, 96]" header />
      <VDivider />
      <VCardText>
        <div v-if="!items.length" class="py-6 text-center text-medium-emphasis">{{ $t('page.abnormal.noItems') }}</div>
        <div v-else class="photo-grid">
          <VCard v-for="it in items" :key="it.id" variant="outlined" class="photo-card">
            <div class="photo-box" :class="{ 'cursor-pointer': it.image_id }" @click="openImage(it)">
              <ProtectedImg v-if="it.image_id" :src="parcelImageUrl(it.image_id)" :alt="it.barcode" class="photo-img" />
              <div v-else class="text-body-small text-disabled text-center px-2">{{ $t('page.abnormal.noPhoto') }}</div>
            </div>
            <div class="px-2 pt-1 d-flex align-center justify-space-between gap-1">
              <span class="text-body-small text-medium-emphasis text-no-wrap">{{ it.started_at.slice(11, 19) }}</span>
              <VChip size="x-small" :color="systemReason(it).color" variant="tonal" label class="text-no-wrap">{{ systemReason(it).text }}</VChip>
            </div>
            <div class="px-2 pb-2 d-flex align-center justify-space-between gap-1">
              <a href="#" class="text-body-small font-weight-medium text-truncate" :title="$t('page.abnormal.openDetail')" @click.prevent="openDetail(it)">{{ it.barcode }}</a>
              <VChip v-if="it.cause" size="x-small" :color="causeColor(it.cause)" variant="tonal" label class="text-no-wrap">{{ $t(`page.abnormal.cause.${it.cause}`) }}</VChip>
            </div>
          </VCard>
        </div>
      </VCardText>
      <VDivider />
      <TablePagination v-model:page="page" v-model:per-page="pageSize" :total="total" :page-sizes="[12, 24, 48, 96]" />
    </VCard>

    <ParcelImageViewer v-model="viewerOpen" :image="viewerImage" />
    <ParcelDetailDialog v-model="detailOpen" :parcel-id="detailId" />
  </div>
</template>

<style scoped>
.photo-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 10px; }
.photo-box { aspect-ratio: 4 / 3; background: rgba(var(--v-theme-on-surface), 0.04); display: flex; align-items: center; justify-content: center; overflow: hidden; }
.photo-box :deep(img), .photo-img { inline-size: 100%; block-size: 100%; object-fit: cover; }
</style>
