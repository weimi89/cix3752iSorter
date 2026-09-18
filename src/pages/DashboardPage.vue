<script setup>
import { useI18n } from 'vue-i18n'
import { useStatusStore } from '@/stores/status'
import { api } from '@/api/http'
import { listen } from '@/api/events'
import { fmtMs, fmtTimeMs, fmtDuration, statusMeta, sourceMeta } from '@/composables/useFormat'
import AppHeader from '@/components/AppHeader.vue'
import PageActions from '@/components/PageActions.vue'
import { toast } from 'vue3-toastify'
import VChart from 'vue-echarts'
import { use } from 'echarts/core'
import { BarChart } from 'echarts/charts'
import { GridComponent, TooltipComponent, LegendComponent } from 'echarts/components'
import { CanvasRenderer } from 'echarts/renderers'

use([BarChart, GridComponent, TooltipComponent, LegendComponent, CanvasRenderer])

const status = useStatusStore()
const { t } = useI18n()
const messages = ref([])
const hourly = ref([])
const busy = ref(false)
const now = ref(Date.now())
let unlistenMsg = null
let clock = null
let hourlyTimer = null

const tracker = computed(() => status.tracker)
const counters = computed(() => tracker.value?.counters || {})
const inFlight = computed(() => tracker.value?.in_flight || [])
const current = computed(() => tracker.value?.current)

const beltState = computed(() => {
  if (!status.devices.belt.connected) return { key: 'page.dashboard.beltOffline', color: 'error', icon: 'tabler-plug-connected-x' }
  if (tracker.value?.head_jam) return { key: 'page.dashboard.beltHeadJam', color: 'warning', icon: 'tabler-alert-triangle' }
  if (tracker.value?.blocked) return { key: 'page.dashboard.beltBlocked', color: 'warning', icon: 'tabler-alert-triangle' }
  return tracker.value?.belt_running
    ? { key: 'page.dashboard.beltRunning', color: 'success', icon: 'tabler-player-play' }
    : { key: 'page.dashboard.beltStopped', color: 'secondary', icon: 'tabler-player-stop' }
})

const control = async fn => {
  busy.value = true
  try { await fn() } catch (e) { toast(e.message, { type: 'error' }) } finally { busy.value = false }
}

// 皮帶啟停放頁首右上角，一顆鈕依目前狀態反向：運轉中顯示「停止」、停著顯示「啟動」
const actions = computed(() => {
  const running = !!tracker.value?.belt_running
  return [
    running
      ? { key: 'beltStop', label: t('page.dashboard.beltStop'), icon: 'tabler-player-stop', color: 'error', variant: 'flat', disabled: busy.value || !status.devices.belt.connected, onClick: () => control(api.beltStop) }
      : { key: 'beltStart', label: t('page.dashboard.beltStart'), icon: 'tabler-player-play', color: 'success', variant: 'flat', disabled: busy.value || !status.devices.belt.connected, onClick: () => control(api.beltStart) },
  ]
})

const loadHourly = async () => {
  try { hourly.value = await api.hourlyStats(12) } catch (e) { console.warn(e) }
}

const chartOption = computed(() => ({
  tooltip: { trigger: 'axis' },
  legend: { top: 0, data: [t('page.dashboard.chart.done'), t('page.dashboard.chart.abnormal')] },
  grid: { left: 40, right: 16, top: 36, bottom: 28 },
  xAxis: { type: 'category', data: hourly.value.map(d => d.hour.slice(11)) },
  yAxis: { type: 'value', minInterval: 1 },
  series: [
    { name: t('page.dashboard.chart.done'), type: 'bar', stack: 'a', data: hourly.value.map(d => d.done), itemStyle: { color: '#76C043' } },
    { name: t('page.dashboard.chart.abnormal'), type: 'bar', stack: 'a', data: hourly.value.map(d => d.abnormal), itemStyle: { color: '#FF4C51' } },
  ],
}))

onMounted(async () => {
  try {
    const d = await api.logs({ limit: 30 })
    messages.value = d.list.map(l => ({ level: l.level, category: l.category, message: l.message, created_at: l.created_at }))
  } catch (e) { console.warn(e) }
  unlistenMsg = listen('system-message', ({ payload }) => {
    messages.value.unshift(payload)
    if (messages.value.length > 60) messages.value.length = 60
  })
  clock = setInterval(() => { now.value = Date.now() }, 500)
  loadHourly()
  hourlyTimer = setInterval(loadHourly, 60_000)
})
onBeforeUnmount(() => { unlistenMsg?.(); clearInterval(clock); clearInterval(hourlyTimer) })

const levelColor = { info: 'info', warn: 'warning', error: 'error' }
// 皮帶線卡片直接顯示運轉／停止／堵塞細狀態（beltState 已含未連線），分揀機與讀碼站只有連線與否
const deviceCards = computed(() => [
  { key: 'belt', icon: 'tabler-arrows-right', ...status.devices.belt, state: beltState.value },
  { key: 'sorter', icon: 'tabler-route', ...status.devices.sorter },
  { key: 'camera', icon: 'tabler-scan', ...status.devices.camera },
])
const latency = computed(() => status.chuteLatency)
const latencyColor = computed(() => {
  const l = latency.value
  if (!l || !l.samples) return 'secondary'
  if (l.p99_ms > l.budget_ms || l.over_budget_pct >= 1) return 'error'
  if (l.p90_ms > l.budget_ms * 0.7) return 'warning'
  return 'success'
})
const stats = computed(() => [
  { key: 'today', icon: 'tabler-packages', color: 'primary', value: status.todayCount, label: t('page.dashboard.today'), hint: t('page.dashboard.todayHint') },
  { key: 'done', icon: 'tabler-circle-check', color: 'success', value: counters.value.done || 0, label: t('page.dashboard.doneSinceStart') },
  { key: 'abnormal', icon: 'tabler-alert-circle', color: 'error', value: counters.value.abnormal || 0, label: t('page.dashboard.abnormal') },
  { key: 'noread', icon: 'tabler-barcode-off', color: 'warning', value: counters.value.noread || 0, label: t('page.dashboard.noread') },
  { key: 'defaulted', icon: 'tabler-arrow-bear-right', color: 'warning', value: counters.value.defaulted || 0, label: t('page.dashboard.defaulted') },
])
</script>

<template>
  <div>
    <AppHeader :title="$t('page.dashboard.title')" :subtitle="$t('page.dashboard.subtitle')" :subtitle-short="$t('page.dashboard.subtitleShort')" icon="tabler-layout-dashboard">
      <template #actions><PageActions :items="actions" /></template>
    </AppHeader>

    <!-- 裝置連線 -->
    <VRow density="compact">
      <VCol v-for="d in deviceCards" :key="d.key" cols="12" md="4">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend>
              <VAvatar :color="d.state ? d.state.color : (d.connected ? 'success' : 'error')" variant="tonal"><VIcon :icon="d.icon" /></VAvatar>
            </template>
            <VCardTitle>{{ $t(`device.${d.key}`) }}</VCardTitle>
            <VCardSubtitle>
              <span v-if="d.state" :class="`text-${d.state.color}`">{{ $t(d.state.key) }}</span>
              <span v-else :class="d.connected ? 'text-success' : 'text-error'">{{ d.connected ? $t('page.dashboard.connected') : $t('page.dashboard.disconnected') }}</span>
              <template v-if="d.since_ms"> · {{ fmtMs(d.since_ms) }}</template>
            </VCardSubtitle>
          </VCardItem>
        </VCard>
      </VCol>
    </VRow>

    <!-- 件數 -->
    <VRow density="compact" class="mt-1">
      <VCol v-for="s in stats" :key="s.key" cols="6" md="4" lg="2">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar :color="s.color" variant="tonal"><VIcon :icon="s.icon" /></VAvatar></template>
            <VCardTitle>{{ s.value }}</VCardTitle>
            <VCardSubtitle>{{ s.label }}<VTooltip v-if="s.hint" activator="parent" location="bottom">{{ s.hint }}</VTooltip></VCardSubtitle>
          </VCardItem>
        </VCard>
      </VCol>
      <VCol cols="6" md="4" lg="2">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar :color="status.print.failed || status.report.failed ? 'error' : 'info'" variant="tonal"><VIcon icon="tabler-stack-2" /></VAvatar></template>
            <VCardTitle class="d-flex ga-2 flex-wrap">
              <VChip size="small" :color="status.print.failed ? 'error' : 'secondary'" variant="tonal" :to="{ name: 'print-jobs' }" label><VIcon icon="tabler-printer" size="14" start />{{ status.print.pending }} / {{ status.print.failed }}</VChip>
              <VChip size="small" :color="status.report.failed ? 'error' : 'secondary'" variant="tonal" :to="{ name: 'report-queue' }" label><VIcon icon="tabler-cloud-upload" size="14" start />{{ status.report.pending }} / {{ status.report.failed }}</VChip>
            </VCardTitle>
            <VCardSubtitle>{{ $t('page.dashboard.queues') }}</VCardSubtitle>
          </VCardItem>
        </VCard>
      </VCol>
    </VRow>

    <VRow density="compact" class="mt-1">
      <VCol cols="12" lg="7">
        <!-- 目前處理中 -->
        <VCard class="card-shadow mb-2">
          <VCardItem>
            <template #prepend><VAvatar color="primary" variant="tonal"><VIcon icon="tabler-focus-2" /></VAvatar></template>
            <VCardTitle>{{ $t('page.dashboard.current') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.dashboard.currentHint') }}</VCardSubtitle>
          </VCardItem>
          <!-- 只放條碼：格口、狀態、小車在下方在途表都有，這裡重複列反而讓各欄位高度對不齊。
               有無包裹都固定同一高度；條碼過長截斷、滑鼠停留看全碼，避免版面隨內容跳動 -->
          <VCardText v-if="current" class="pt-0 d-flex align-center current-body">
            <div class="text-headline-small font-weight-medium selectable text-truncate" :title="current.barcode || 'NoRead'">{{ current.barcode || 'NoRead' }}</div>
          </VCardText>
          <VCardText v-else class="pt-0 d-flex align-center text-medium-emphasis current-body">{{ $t('page.dashboard.noCurrent') }}</VCardText>
        </VCard>

        <!-- 在途 -->
        <VCard class="card-shadow">
          <VCardItem>
            <template #prepend><VAvatar color="info" variant="tonal"><VIcon icon="tabler-truck-loading" /></VAvatar></template>
            <VCardTitle>{{ $t('page.dashboard.inFlight') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.dashboard.inFlightHint') }}</VCardSubtitle>
            <template #append><span class="text-title-large font-weight-bold text-info">{{ inFlight.length }}</span></template>
          </VCardItem>
          <VDivider />
          <VTable hover class="table-cards inflight-table">
            <thead><tr>
              <!-- 定長的欄位（時間、格口、來源、狀態、小車、經過）給固定寬，剩下的留給條碼；
                   表格鎖 table-layout: fixed，條碼欄放不下就截斷（滑鼠停留看全碼），不讓整張表橫向捲動把最後一欄藏掉。
                   在途列只會存在幾秒到幾分鐘，開始時間只顯示時分秒，日期省下的寬度留給條碼 -->
              <th class="text-center" style="width: 118px;">{{ $t('parcel.startedAt') }}</th><th class="text-center">{{ $t('parcel.barcode') }}</th><th class="text-center" style="width: 56px;">{{ $t('parcel.chute') }}</th><th class="text-center" style="width: 104px;">{{ $t('parcel.source') }}</th><th class="text-center" style="width: 84px;">{{ $t('parcel.status') }}</th><th class="text-center" style="width: 56px;">{{ $t('parcel.cart') }}</th><th class="text-center" style="width: 92px;">{{ $t('page.dashboard.elapsed') }}</th>
            </tr></thead>
            <tbody>
              <tr v-if="!inFlight.length"><td colspan="7"><div class="py-2 d-flex align-center justify-center"><VIcon icon="tabler-alert-circle" size="20" class="me-1" /><span class="text-md">{{ $t('common.noData') }}</span></div></td></tr>
              <tr v-for="p in inFlight" :key="p.key">
                <td :data-label="$t('parcel.startedAt')" class="text-center text-no-wrap">{{ fmtTimeMs(p.p_ms) }}</td>
                <td :data-label="$t('parcel.barcode')" class="text-center"><div class="barcode-cell selectable text-truncate mx-auto" :title="p.barcode || ''">{{ p.barcode || '—' }}</div></td>
                <td :data-label="$t('parcel.chute')" class="text-center">{{ p.chute?.code || '—' }}</td>
                <td :data-label="$t('parcel.source')" class="text-center"><VChip v-if="p.chute" size="x-small" :color="sourceMeta(p.chute.source).color" variant="tonal" label>{{ $t(sourceMeta(p.chute.source).key) }}</VChip></td>
                <td :data-label="$t('parcel.status')" class="text-center"><VChip size="x-small" :color="statusMeta(p.status).color" label>{{ $t(statusMeta(p.status).key) }}</VChip></td>
                <td :data-label="$t('parcel.cart')" class="text-center">{{ p.cart ?? '' }}</td>
                <td :data-label="$t('page.dashboard.elapsed')" class="text-center text-no-wrap">{{ fmtDuration(now - p.p_ms) }}</td>
              </tr>
            </tbody>
          </VTable>
        </VCard>
      </VCol>

      <VCol cols="12" lg="5">
        <!-- 中介機查格口耗時：變慢時現場能提早發現，不用等到一堆件走預設口 -->
        <VCard class="card-shadow mb-2">
          <VCardItem>
            <template #prepend><VAvatar :color="latencyColor" variant="tonal"><VIcon icon="tabler-clock-bolt" /></VAvatar></template>
            <VCardTitle>{{ $t('page.dashboard.latency') }}<VTooltip activator="parent" location="bottom">{{ $t('page.dashboard.latencyHint') }}</VTooltip></VCardTitle>
            <VCardSubtitle v-if="latency && latency.samples">{{ latency.samples }} {{ $t('page.dashboard.latencySamples') }} · {{ $t('page.dashboard.latencyOver') }} {{ latency.over_budget_pct.toFixed(1) }}%</VCardSubtitle>
            <VCardSubtitle v-else>{{ $t('page.dashboard.latencyNone') }}</VCardSubtitle>
            <template #append>
              <div v-if="latency && latency.samples" class="d-flex ga-4 text-center">
                <div v-for="q in ['p50', 'p90', 'p99']" :key="q">
                  <div class="text-title-medium font-weight-bold" :class="latency[`${q}_ms`] > latency.budget_ms ? 'text-error' : ''">{{ latency[`${q}_ms`] }}<span class="text-body-small text-medium-emphasis">ms</span></div>
                  <div class="text-body-small text-medium-emphasis">{{ q }}</div>
                </div>
              </div>
            </template>
          </VCardItem>
        </VCard>
        <VCard class="card-shadow mb-2">
          <VCardItem>
            <template #prepend><VAvatar color="primary" variant="tonal"><VIcon icon="tabler-chart-bar" /></VAvatar></template>
            <VCardTitle>{{ $t('page.dashboard.hourlyChart') }}</VCardTitle>
          </VCardItem>
          <VCardText class="pt-0"><VChart :option="chartOption" autoresize style="height: 200px" /></VCardText>
        </VCard>
        <VCard class="card-shadow">
          <VCardItem>
            <template #prepend><VAvatar color="secondary" variant="tonal"><VIcon icon="tabler-bell-ringing" /></VAvatar></template>
            <VCardTitle>{{ $t('page.dashboard.messages') }}</VCardTitle>
          </VCardItem>
          <VDivider />
          <VList density="compact" class="py-1" style="max-height: 440px; overflow: auto">
            <VListItem v-for="(m, i) in messages" :key="i">
              <template #prepend>
                <VIcon :icon="m.level === 'error' ? 'tabler-circle-x' : m.level === 'warn' ? 'tabler-alert-triangle' : 'tabler-info-circle'" :color="levelColor[m.level] || 'info'" size="18" class="me-2" />
              </template>
              <VListItemTitle class="text-body-medium">{{ m.message }}</VListItemTitle>
              <VListItemSubtitle class="text-body-small">{{ $t(`log.cat.${m.category}`, m.category) }} · {{ m.created_at }}</VListItemSubtitle>
            </VListItem>
          </VList>
        </VCard>
      </VCol>
    </VRow>
  </div>
</template>

<style scoped>
/* 「目前處理中」有無包裹都撐同一高度，內容變動不影響下方表格位置 */
.current-body { min-height: 64px; }
.barcode-cell { max-width: 160px; }
/* 欄寬照表頭指定，不隨內容撐開；手機卡片式（table-cards）會改成 block，這條不影響 */
@media (min-width: 640px) {
  .inflight-table :deep(table) { table-layout: fixed; inline-size: 100%; }
}
</style>
