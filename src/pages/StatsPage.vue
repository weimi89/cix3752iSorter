<script setup>
import { useI18n } from 'vue-i18n'
import { useTheme } from 'vuetify'
import { api } from '@/api/http'
import { onConnection } from '@/api/events'
import { fmtDate, fmtDuration, fmtTime, statusMeta, sourceMeta } from '@/composables/useFormat'
import AppHeader from '@/components/AppHeader.vue'
import AppDatePicker from '@/components/AppDatePicker.vue'
import PageActions from '@/components/PageActions.vue'
import VChart from 'vue-echarts'
import { use } from 'echarts/core'
import { BarChart, LineChart, HeatmapChart } from 'echarts/charts'
import { GridComponent, TooltipComponent, LegendComponent, VisualMapComponent, MarkLineComponent } from 'echarts/components'
import { CanvasRenderer } from 'echarts/renderers'

use([BarChart, LineChart, HeatmapChart, GridComponent, TooltipComponent, LegendComponent, VisualMapComponent, MarkLineComponent, CanvasRenderer])

const { t, te } = useI18n()
const theme = useTheme()
const color = name => theme.current.value.colors[name]
const hexToRgba = (hex, alpha) => {
  const h = hex.replace('#', '')
  return `rgba(${parseInt(h.slice(0, 2), 16)},${parseInt(h.slice(2, 4), 16)},${parseInt(h.slice(4, 6), 16)},${alpha})`
}

// 日期區間：預設今日；快選鍵只是把起訖填成對應日期，手動改日期就沒有快選亮著
const dateOffset = days => { const d = new Date(); d.setDate(d.getDate() + days); return fmtDate(d) }
const startDate = ref(dateOffset(0))
const endDate = ref(dateOffset(0))
const quickRange = computed({
  get() {
    if (endDate.value !== dateOffset(0)) return null
    if (startDate.value === dateOffset(0)) return 'today'
    if (startDate.value === dateOffset(-6)) return '7d'
    if (startDate.value === dateOffset(-29)) return '30d'
    return null
  },
  set(v) {
    const days = { today: 1, '7d': 7, '30d': 30 }[v]
    if (!days) return
    endDate.value = dateOffset(0)
    startDate.value = dateOffset(-(days - 1))
  },
})
const isSingleDay = computed(() => startDate.value === endDate.value)

const data = ref(null)
const loading = ref(false)
const errorMsg = ref('')
const lastLoadedAt = ref(null)

// 背景自動更新只在區間包含今天時進行（過往區間的數字不會變），且分頁在前景才跑；
// 自動更新不亮整頁的載入狀態，避免每 30 秒閃一次
const AUTO_REFRESH_MS = 30_000
const includesToday = computed(() => endDate.value >= dateOffset(0))
let inFlight = false
const load = async (silent = false) => {
  if (inFlight) return
  inFlight = true
  if (!silent) loading.value = true
  errorMsg.value = ''
  try {
    data.value = await api.statsOverview(startDate.value, endDate.value)
    lastLoadedAt.value = Date.now()
  } catch (e) {
    errorMsg.value = e.message
  } finally {
    loading.value = false
    inFlight = false
  }
}
const reload = () => load(false)
const autoRefresh = () => {
  if (document.hidden || !includesToday.value) return
  load(true)
}
// 分頁從背景回到前景時，超過一輪沒更新就立刻補一次
const onVisibility = () => {
  if (document.hidden || !includesToday.value) return
  if (Date.now() - (lastLoadedAt.value || 0) >= AUTO_REFRESH_MS) load(true)
}
let autoTimer = null
let unlistenConnection = null
let disconnectedAt = null
watch([startDate, endDate], reload)
onMounted(() => {
  reload()
  autoTimer = setInterval(autoRefresh, AUTO_REFRESH_MS)
  document.addEventListener('visibilitychange', onVisibility)
  // 與後端斷線又接上（後端重啟、網路閃斷）時補一次，不用等下一輪。
  // 只補「上次載入之後才斷掉」的那種：進頁當下事件串流可能還沒接上，那次接上不是重連，
  // 首次載入已經拿到最新資料，不能再抓一次
  unlistenConnection = onConnection(connected => {
    if (!connected) { disconnectedAt = Date.now(); return }
    if (disconnectedAt && lastLoadedAt.value && lastLoadedAt.value < disconnectedAt) autoRefresh()
    disconnectedAt = null
  })
})
onBeforeUnmount(() => {
  clearInterval(autoTimer)
  document.removeEventListener('visibilitychange', onVisibility)
  unlistenConnection?.()
})

const actions = computed(() => [
  { key: 'reload', label: t('common.reload'), icon: 'tabler-refresh', color: 'primary', variant: 'flat', loading: loading.value, onClick: reload },
])

// 區間起點早於包裹表最早一筆 → 分項統計（格口／來源／時段…）只涵蓋還留著的資料
const partialNote = computed(() => {
  const d = data.value
  if (!d || !d.parcels_since || d.parcels_since <= d.from) return ''
  return t('page.stats.partialNote', { since: d.parcels_since, days: d.retention_days })
})

const kpiCards = computed(() => {
  const k = data.value?.kpi
  return [
    { key: 'today', label: t('page.stats.today'), icon: 'tabler-calendar-event', color: 'primary', b: k?.today, primary: true },
    { key: 'yesterday', label: t('page.stats.yesterday'), icon: 'tabler-calendar-minus', color: 'info', b: k?.yesterday },
    { key: 'last7', label: t('page.stats.last7Days'), icon: 'tabler-calendar-week', color: 'success', b: k?.last7 },
    { key: 'last30', label: t('page.stats.last30Days'), icon: 'tabler-calendar-month', color: 'warning', b: k?.last30 },
  ]
})

const range = computed(() => data.value?.range || { total: 0, done: 0, noread: 0, defaulted: 0, abnormal: 0 })
const doneRate = computed(() => (range.value.total ? Math.round((range.value.done / range.value.total) * 1000) / 10 : null))
const rangeItems = computed(() => [
  { key: 'done', label: t('page.stats.done'), value: range.value.done, color: 'success', icon: 'tabler-circle-check' },
  { key: 'abnormal', label: t('page.stats.abnormal'), value: range.value.abnormal, color: 'error', icon: 'tabler-alert-circle' },
  { key: 'noread', label: t('page.stats.noread'), value: range.value.noread, color: 'warning', icon: 'tabler-barcode-off' },
  { key: 'defaulted', label: t('page.stats.defaulted'), value: range.value.defaulted, color: 'warning', icon: 'tabler-arrow-bear-right' },
])

const pct = (n, max) => (max > 0 ? Math.round((n / max) * 100) : 0)
// 佔比：非 0 但不到 1% 顯示一位小數，不要出現「1 (0%)」
const share = (n, total) => {
  if (!total || !n) return 0
  const v = (n / total) * 100
  return v < 1 ? Math.round(v * 10) / 10 : Math.round(v)
}

// 趨勢：多日看每日、單日看 24 小時；完成／異常疊在同一根柱子上，一眼看得出異常佔比
const axisStyle = () => ({
  axisLine: { lineStyle: { color: hexToRgba(color('on-surface'), 0.12) } },
  axisLabel: { color: hexToRgba(color('on-surface'), 0.6), fontSize: 11 },
  axisTick: { show: false },
})
const stackedBarOption = (labels, done, abnormal) => ({
  tooltip: { trigger: 'axis' },
  legend: { top: 0, data: [t('page.stats.done'), t('page.stats.abnormal')], textStyle: { color: hexToRgba(color('on-surface'), 0.7) } },
  grid: { left: 40, right: 16, top: 36, bottom: 28 },
  xAxis: { type: 'category', data: labels, ...axisStyle() },
  yAxis: { type: 'value', minInterval: 1, axisLine: { show: false }, splitLine: { lineStyle: { color: hexToRgba(color('on-surface'), 0.06) } }, axisLabel: axisStyle().axisLabel, axisTick: { show: false } },
  series: [
    { name: t('page.stats.done'), type: 'bar', stack: 'a', data: done, itemStyle: { color: color('success') } },
    { name: t('page.stats.abnormal'), type: 'bar', stack: 'a', data: abnormal, itemStyle: { color: color('error') } },
  ],
})
const trendOption = computed(() => {
  const d = data.value
  if (!d) return stackedBarOption([], [], [])
  if (isSingleDay.value) {
    return stackedBarOption(d.hourly.map(h => String(h.hour).padStart(2, '0')), d.hourly.map(h => h.done), d.hourly.map(h => h.abnormal))
  }
  return stackedBarOption(d.daily.map(r => r.day.slice(5)), d.daily.map(r => r.done), d.daily.map(r => r.abnormal))
})
const hourlyOption = computed(() => {
  const h = data.value?.hourly || []
  return stackedBarOption(h.map(x => String(x.hour).padStart(2, '0')), h.map(x => x.done), h.map(x => x.abnormal))
})

const chutes = computed(() => data.value?.by_chute || [])
const chutesMax = computed(() => chutes.value.reduce((a, b) => Math.max(a, b.total), 0))
const chutesTotal = computed(() => chutes.value.reduce((a, b) => a + b.total, 0))

const sources = computed(() => data.value?.by_source || [])
const sourcesTotal = computed(() => sources.value.reduce((a, b) => a + b.count, 0))
const statuses = computed(() => data.value?.by_status || [])
const statusesTotal = computed(() => statuses.value.reduce((a, b) => a + b.count, 0))

const travel = computed(() => data.value?.travel)
const travelItems = computed(() => {
  const tr = travel.value
  if (!tr || !tr.samples) return []
  return [
    { key: 'p50', label: 'p50', value: tr.p50_ms },
    { key: 'p90', label: 'p90', value: tr.p90_ms },
    { key: 'p99', label: 'p99', value: tr.p99_ms },
    { key: 'avg', label: t('page.stats.avg'), value: tr.avg_ms },
    { key: 'max', label: t('page.stats.max'), value: tr.max_ms },
  ]
})

const print = computed(() => data.value?.print)
const printFailRate = computed(() => {
  const p = print.value
  const settled = (p?.done || 0) + (p?.failed || 0)
  return settled ? Math.round((p.failed / settled) * 1000) / 10 : null
})
// 印表機列照格口表的順序排（埠位字串排序會讓右側 1-3.x 跑到左側 1-8.x 前面）
const printers = computed(() => {
  const order = new Map(chutes.value.map((c, i) => [c.code, i]))
  const rank = p => (p.chutes.length ? Math.min(...p.chutes.map(c => order.get(c) ?? 999)) : 999)
  return [...(print.value?.by_printer || [])].sort((a, b) => rank(a) - rank(b) || a.printer_port.localeCompare(b.printer_port))
})
const printersMax = computed(() => printers.value.reduce((a, b) => Math.max(a, b.total), 0))
const report = computed(() => data.value?.report)
const reportFailRate = computed(() => {
  const r = report.value
  const settled = (r?.success || 0) + (r?.failed || 0)
  return settled ? Math.round((r.failed / settled) * 1000) / 10 : null
})

const compare = computed(() => data.value?.compare || [])

// 異常原因：代碼對到文案，沒對到的顯示代碼本身（中介機日後新增的錯誤碼也看得懂）
const reasons = computed(() => data.value?.reasons || [])
const reasonsTotal = computed(() => reasons.value.reduce((a, b) => a + b.count, 0))
const reasonLabel = key => {
  const k = `page.stats.reason.${key}`
  return te(k) ? t(k) : key
}

const jams = computed(() => data.value?.jams)
const jamModulesMax = computed(() => (jams.value?.by_module || []).reduce((a, b) => Math.max(a, b.count), 0))
const jamsHourOption = computed(() => {
  const h = jams.value?.by_hour || []
  return {
    tooltip: { trigger: 'axis' },
    grid: { left: 28, right: 8, top: 8, bottom: 22 },
    xAxis: { type: 'category', data: h.map((_, i) => String(i).padStart(2, '0')), ...axisStyle(), axisLabel: { ...axisStyle().axisLabel, fontSize: 9, interval: 1 } },
    yAxis: { type: 'value', minInterval: 1, axisLine: { show: false }, splitLine: { lineStyle: { color: hexToRgba(color('on-surface'), 0.06) } }, axisLabel: { ...axisStyle().axisLabel, fontSize: 9 }, axisTick: { show: false } },
    series: [{ type: 'bar', data: h, itemStyle: { color: color('warning') } }],
  }
})

// 各段耗時：超過預算的段標紅
const stages = computed(() => data.value?.stages || [])
const stageOver = st => st.budget_ms !== null && st.samples > 0 && st.over_budget > 0
const travelByChute = computed(() => {
  // 照格口表順序排，不照代碼字母
  const order = new Map(chutes.value.map((c, i) => [c.code, i]))
  return [...(data.value?.travel_by_chute || [])].sort((a, b) => (order.get(a.code) ?? 999) - (order.get(b.code) ?? 999))
})
const travelByChuteMax = computed(() => travelByChute.value.reduce((a, b) => Math.max(a, b.p90_ms), 0))

const devices = computed(() => data.value?.devices || [])
const deviceIcon = { belt: 'tabler-arrows-right', sorter: 'tabler-route', camera: 'tabler-scan' }
// 斷線時間：秒以下顯示毫秒、一分鐘以上顯示分秒
const fmtOutage = ms => {
  if (!ms) return '—'
  if (ms < 1000) return `${ms} ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`
  const m = Math.floor(ms / 60_000)
  const sec = Math.round((ms % 60_000) / 1000)
  return m >= 60 ? `${Math.floor(m / 60)} h ${m % 60} m` : `${m} m ${sec} s`
}

const duplicates = computed(() => data.value?.duplicates)

// 裝置斷線依時段：三種裝置疊在同一根柱子上
const devicesHourOption = computed(() => ({
  tooltip: { trigger: 'axis' },
  legend: { top: 0, data: devices.value.map(d => t('device.' + d.device)), textStyle: { color: hexToRgba(color('on-surface'), 0.7), fontSize: 11 } },
  grid: { left: 28, right: 8, top: 28, bottom: 22 },
  xAxis: { type: 'category', data: Array.from({ length: 24 }, (_, i) => String(i).padStart(2, '0')), ...axisStyle(), axisLabel: { ...axisStyle().axisLabel, fontSize: 9, interval: 1 } },
  yAxis: { type: 'value', minInterval: 1, axisLine: { show: false }, splitLine: { lineStyle: { color: hexToRgba(color('on-surface'), 0.06) } }, axisLabel: { ...axisStyle().axisLabel, fontSize: 9 }, axisTick: { show: false } },
  series: devices.value.map((d, i) => ({ name: t('device.' + d.device), type: 'bar', stack: 'a', data: d.by_hour, itemStyle: { color: [color('warning'), color('error'), color('info')][i] } })),
}))

// 讀碼失敗率：多日看每天、單日看每小時；柱=件數、線=比例
const noreadPoints = computed(() => {
  const d = data.value
  if (!d) return []
  const rows = isSingleDay.value ? d.hourly.map(h => ({ label: String(h.hour).padStart(2, '0'), total: h.total, noread: h.noread })) : d.daily.map(r => ({ label: r.day.slice(5), total: r.total, noread: r.noread }))
  return rows.map(r => ({ ...r, rate: r.total ? Math.round((r.noread / r.total) * 1000) / 10 : null }))
})
// 讀碼失敗依包裹長度（光電長度單位）：短件失敗率高得多，現場調讀碼站後拿這張圖對照
const lengthRows = computed(() => (data.value?.noread_by_length || []).map(b => ({ ...b, rate: b.total ? Math.round((b.noread / b.total) * 1000) / 10 : null })))
const noreadLengthOption = computed(() => ({
  tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' }, formatter: ps => ps.map(p => `${p.marker}${p.seriesName}：${p.value ?? '—'}${p.seriesIndex === 1 ? '%' : ''}`).join('<br>') },
  legend: { top: 0, data: [t('page.stats.lengthTotalSeries'), t('page.stats.noreadRateSeries')], textStyle: { color: hexToRgba(color('on-surface'), 0.7) } },
  grid: { left: 44, right: 44, top: 36, bottom: 28 },
  xAxis: { type: 'category', data: lengthRows.value.map(r => r.bucket), ...axisStyle() },
  yAxis: [
    { type: 'value', ...axisStyle(), splitLine: { lineStyle: { color: hexToRgba(color('on-surface'), 0.08) } } },
    { type: 'value', ...axisStyle(), axisLabel: { formatter: '{value}%', color: hexToRgba(color('on-surface'), 0.6) }, splitLine: { show: false } },
  ],
  series: [
    { name: t('page.stats.lengthTotalSeries'), type: 'bar', data: lengthRows.value.map(r => r.total), itemStyle: { color: hexToRgba(color('info'), 0.5) } },
    { name: t('page.stats.noreadRateSeries'), type: 'bar', yAxisIndex: 1, data: lengthRows.value.map(r => r.rate), itemStyle: { color: color('warning') } },
  ],
}))
const noreadOption = computed(() => ({
  tooltip: { trigger: 'axis' },
  legend: { top: 0, data: [t('page.stats.noreadCountSeries'), t('page.stats.noreadRateSeries')], textStyle: { color: hexToRgba(color('on-surface'), 0.7) } },
  grid: { left: 40, right: 44, top: 36, bottom: 28 },
  xAxis: { type: 'category', data: noreadPoints.value.map(p => p.label), ...axisStyle() },
  yAxis: [
    { type: 'value', minInterval: 1, axisLine: { show: false }, splitLine: { lineStyle: { color: hexToRgba(color('on-surface'), 0.06) } }, axisLabel: axisStyle().axisLabel, axisTick: { show: false } },
    { type: 'value', axisLine: { show: false }, splitLine: { show: false }, axisLabel: { ...axisStyle().axisLabel, formatter: '{value}%' }, axisTick: { show: false } },
  ],
  series: [
    { name: t('page.stats.noreadCountSeries'), type: 'bar', data: noreadPoints.value.map(p => p.noread), itemStyle: { color: hexToRgba(color('warning'), 0.6) } },
    { name: t('page.stats.noreadRateSeries'), type: 'line', yAxisIndex: 1, smooth: true, data: noreadPoints.value.map(p => p.rate), itemStyle: { color: color('warning') }, lineStyle: { width: 2, color: color('warning') }, connectNulls: true },
  ],
}))

// 小車異常率：明顯高於平均（1.5 倍且至少多 1 件）標紅
const carts = computed(() => data.value?.by_cart || [])
const cartAvgRate = computed(() => {
  const total = carts.value.reduce((a, b) => a + b.total, 0)
  const abn = carts.value.reduce((a, b) => a + b.abnormal, 0)
  return total ? (abn / total) * 100 : 0
})
const cartRate = c => (c.total ? (c.abnormal / c.total) * 100 : 0)
const cartHot = c => c.total >= 20 && c.abnormal >= 2 && cartRate(c) > cartAvgRate.value * 1.5 && cartRate(c) - cartAvgRate.value >= 1
const cartOption = computed(() => ({
  tooltip: { trigger: 'axis', formatter: ps => { const c = carts.value[ps[0].dataIndex]; return `${t('page.stats.byCart')} ${c.cart}${t('page.stats.cartUnit')}<br>${c.abnormal} / ${c.total} = ${cartRate(c).toFixed(1)}%` } },
  grid: { left: 40, right: 8, top: 12, bottom: 24 },
  xAxis: { type: 'category', data: carts.value.map(c => c.cart), ...axisStyle(), axisLabel: { ...axisStyle().axisLabel, fontSize: 9, interval: 0 } },
  yAxis: { type: 'value', axisLine: { show: false }, splitLine: { lineStyle: { color: hexToRgba(color('on-surface'), 0.06) } }, axisLabel: { ...axisStyle().axisLabel, formatter: '{value}%' }, axisTick: { show: false } },
  series: [{
    type: 'bar',
    data: carts.value.map(c => ({ value: Math.round(cartRate(c) * 10) / 10, itemStyle: { color: cartHot(c) ? color('error') : color('info') } })),
    markLine: { silent: true, symbol: 'none', lineStyle: { color: hexToRgba(color('on-surface'), 0.4), type: 'dashed' }, label: { formatter: `${t('page.stats.cartAvg')} ${cartAvgRate.value.toFixed(1)}%`, fontSize: 10, color: hexToRgba(color('on-surface'), 0.6) }, data: [{ yAxis: Math.round(cartAvgRate.value * 10) / 10 }] },
  }],
}))

const weekdays = computed(() => ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'].map(k => t(`page.stats.wd.${k}`)))
const heatmapCells = computed(() => data.value?.heatmap || [])
const heatmapMax = computed(() => heatmapCells.value.reduce((a, b) => Math.max(a, b.count), 0) || 1)
const heatmapOption = computed(() => ({
  tooltip: {
    position: 'top',
    formatter: ({ value: [hour, wd, count] }) => `${weekdays.value[wd]} ${String(hour).padStart(2, '0')}:00<br>${count}`,
  },
  grid: { left: 50, right: 16, top: 8, bottom: 24 },
  xAxis: { type: 'category', data: Array.from({ length: 24 }, (_, i) => String(i).padStart(2, '0')), splitArea: { show: true }, axisLabel: { color: hexToRgba(color('on-surface'), 0.6), fontSize: 10 } },
  yAxis: { type: 'category', data: weekdays.value, splitArea: { show: true }, axisLabel: { color: hexToRgba(color('on-surface'), 0.6), fontSize: 11 } },
  visualMap: { min: 0, max: heatmapMax.value, show: false, inRange: { color: [hexToRgba(color('primary'), 0.08), color('primary')] } },
  series: [{ type: 'heatmap', data: heatmapCells.value.map(c => [c.hour, c.weekday, c.count]), emphasis: { itemStyle: { shadowBlur: 8, shadowColor: hexToRgba(color('primary'), 0.4) } } }],
}))
</script>

<template>
  <div>
    <AppHeader :title="$t('page.stats.title')" :subtitle="$t('page.stats.subtitle')" :subtitle-short="$t('page.stats.subtitleShort')" icon="tabler-chart-pie">
      <template #actions><PageActions :items="actions" /></template>
    </AppHeader>

    <VAlert v-if="errorMsg" type="error" variant="tonal" class="mb-3">{{ errorMsg }}</VAlert>

    <!-- 概況：今日／昨日／近 7 天／近 30 天，今日主色描邊 -->
    <VRow density="compact">
      <VCol v-for="c in kpiCards" :key="c.key" cols="6" md="3">
        <VCard class="card-shadow kpi-card h-100" :class="{ 'kpi-card--primary': c.primary }">
          <VCardText class="d-flex align-center gap-3">
            <VAvatar :color="c.color" :variant="c.primary ? 'flat' : 'tonal'" :size="c.primary ? 44 : 40"><VIcon :icon="c.icon" :size="c.primary ? 24 : 20" /></VAvatar>
            <div class="flex-grow-1" style="min-width: 0">
              <div class="text-body-small text-medium-emphasis">{{ c.label }}</div>
              <div class="kpi-card__count font-weight-bold" :class="c.primary ? 'text-primary' : ''">{{ c.b?.total ?? 0 }}</div>
              <!-- 兩個數字各自不拆行，手機放不下時整組換到下一行，不會拆成「異／常」 -->
              <div class="text-body-small text-medium-emphasis mt-1 d-flex flex-wrap gap-x-2">
                <span class="text-success text-no-wrap">{{ $t('page.stats.done') }} {{ c.b?.done ?? 0 }}</span>
                <span class="text-no-wrap" :class="c.b?.abnormal ? 'text-error' : ''">{{ $t('page.stats.abnormal') }} {{ c.b?.abnormal ?? 0 }}</span>
              </div>
            </div>
          </VCardText>
        </VCard>
      </VCol>
    </VRow>

    <!-- 區間選擇 + 區間總覽 -->
    <VCard class="mt-3 card-shadow">
      <VCardText>
        <div class="d-flex flex-wrap align-center gap-x-4 gap-y-3">
          <div class="stats-date-row d-flex align-center gap-2 flex-nowrap">
            <div class="stats-date-field"><AppDatePicker v-model="startDate" :label="$t('page.stats.startDate')" :max="endDate" /></div>
            <span class="text-medium-emphasis">~</span>
            <div class="stats-date-field"><AppDatePicker v-model="endDate" :label="$t('page.stats.endDate')" :min="startDate" :max="dateOffset(0)" /></div>
          </div>
          <VBtnToggle v-model="quickRange" density="comfortable" color="primary" variant="tonal" divided>
            <VBtn value="today" size="small">{{ $t('page.stats.today') }}</VBtn>
            <VBtn value="7d" size="small">{{ $t('page.stats.last7Days') }}</VBtn>
            <VBtn value="30d" size="small">{{ $t('page.stats.last30Days') }}</VBtn>
          </VBtnToggle>
          <div class="text-body-small text-medium-emphasis stats-refresh-note">
            <template v-if="lastLoadedAt">{{ $t('page.stats.lastUpdated', { time: fmtTime(lastLoadedAt) }) }} · </template>{{ includesToday ? $t('page.stats.autoRefreshOn', { sec: AUTO_REFRESH_MS / 1000 }) : $t('page.stats.autoRefreshOff') }}
          </div>
          <VSpacer />
          <div class="d-flex align-center gap-4">
            <div class="text-center">
              <div class="text-body-small text-medium-emphasis">{{ $t('page.stats.rangeTotal') }}</div>
              <div class="text-headline-small font-weight-bold text-primary">{{ range.total }}</div>
            </div>
            <div class="text-center range-rate">
              <div class="text-body-small text-medium-emphasis">{{ $t('page.stats.doneRate') }}<VTooltip activator="parent" location="bottom">{{ $t('page.stats.doneRateHint') }}</VTooltip></div>
              <div class="text-headline-small font-weight-bold" :class="doneRate === null ? 'text-medium-emphasis' : doneRate >= 99 ? 'text-success' : doneRate >= 95 ? 'text-warning' : 'text-error'">{{ doneRate === null ? '—' : doneRate + '%' }}</div>
            </div>
          </div>
        </div>
        <div class="d-flex flex-wrap gap-2 mt-3">
          <VChip v-for="i in rangeItems" :key="i.key" :color="i.value ? i.color : 'secondary'" variant="tonal" size="small" label>
            <VIcon :icon="i.icon" size="14" start />{{ i.label }} {{ i.value }}
          </VChip>
        </div>
        <VAlert v-if="partialNote" type="info" variant="tonal" density="compact" class="mt-3">{{ partialNote }}</VAlert>
      </VCardText>
    </VCard>

    <!-- 趨勢 + 歷史比對 -->
    <VRow density="compact" class="mt-3">
      <VCol cols="12" md="7">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar color="primary" variant="tonal"><VIcon icon="tabler-chart-bar" /></VAvatar></template>
            <VCardTitle>{{ isSingleDay ? $t('page.stats.hourlyTrend') : $t('page.stats.dailyTrend') }}</VCardTitle>
            <VCardSubtitle>{{ isSingleDay ? $t('page.stats.hourlyTrendHint') : $t('page.stats.dailyTrendHint') }}</VCardSubtitle>
          </VCardItem>
          <VCardText class="pt-0"><VChart :option="trendOption" autoresize style="height: 260px" /></VCardText>
        </VCard>
      </VCol>
      <VCol cols="12" md="5">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar color="info" variant="tonal"><VIcon icon="tabler-trending-up" /></VAvatar></template>
            <VCardTitle>{{ $t('page.stats.compareTitle') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.stats.compareHint') }}</VCardSubtitle>
          </VCardItem>
          <VDivider />
          <VCardText>
            <div v-for="c in compare" :key="c.label" class="compare-row">
              <div class="compare-row__label">{{ c.label === 'week' ? $t('page.stats.compareWeek') : $t('page.stats.compareMonth') }}</div>
              <div class="compare-row__nums">
                <span class="text-headline-small font-weight-bold">{{ c.current }}</span>
                <span class="text-body-small text-medium-emphasis ms-1">vs {{ c.previous }}</span>
              </div>
              <div class="compare-row__delta">
                <span v-if="c.delta_ratio === null" class="text-body-small text-medium-emphasis">—</span>
                <template v-else>
                  <VIcon :icon="c.delta_ratio >= 0 ? 'tabler-arrow-up-right' : 'tabler-arrow-down-right'" :color="c.delta_ratio >= 0 ? 'success' : 'error'" size="18" />
                  <span :class="c.delta_ratio >= 0 ? 'text-success' : 'text-error'" class="font-weight-bold ms-1">{{ (c.delta_ratio * 100).toFixed(1) }}%</span>
                </template>
              </div>
            </div>
            <!-- 通過時間：正常完成件從上線到落格口 -->
            <VDivider class="my-3" />
            <div class="d-flex align-center justify-space-between mb-2">
              <div class="text-body-medium font-weight-medium">{{ $t('page.stats.travel') }}<VTooltip activator="parent" location="bottom">{{ $t('page.stats.travelHint') }}</VTooltip></div>
              <div class="text-body-small text-medium-emphasis">{{ travel?.samples || 0 }} {{ $t('page.stats.samples') }}</div>
            </div>
            <div v-if="travelItems.length" class="d-flex flex-wrap ga-4">
              <div v-for="i in travelItems" :key="i.key" class="text-center">
                <div class="text-title-medium font-weight-bold">{{ fmtDuration(i.value) }}</div>
                <div class="text-body-small text-medium-emphasis">{{ i.label }}</div>
              </div>
            </div>
            <div v-else class="text-body-small text-medium-emphasis">{{ $t('common.noData') }}</div>
          </VCardText>
        </VCard>
      </VCol>
    </VRow>

    <!-- 依格口 + 時段分布（單日時趨勢圖已是小時圖，時段分布不再重複顯示） -->
    <VRow density="compact" class="mt-3">
      <VCol cols="12" :md="isSingleDay ? 12 : 6">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar color="info" variant="tonal"><VIcon icon="tabler-route" /></VAvatar></template>
            <VCardTitle>{{ $t('page.stats.byChute') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.stats.byChuteHint') }}</VCardSubtitle>
          </VCardItem>
          <VDivider />
          <VCardText>
            <div v-if="!chutes.length" class="empty-state"><VIcon icon="tabler-route-off" size="40" class="empty-state__icon" /><div class="empty-state__text">{{ $t('common.noData') }}</div></div>
            <div v-else class="stat-rows">
              <div v-for="c in chutes" :key="c.code" class="stat-row" :class="{ 'stat-row--zero': c.total === 0 }">
                <div class="stat-row__label stat-row__label--wide" :title="c.label">{{ c.code }}<span v-if="c.label" class="text-medium-emphasis ms-1">{{ c.label }}</span></div>
                <div class="stat-row__bar stat-row__bar--split">
                  <div class="stat-row__fill stat-row__fill--success" :style="{ inlineSize: pct(c.done, chutesMax) + '%' }" />
                  <div class="stat-row__fill stat-row__fill--error" :style="{ inlineSize: pct(c.abnormal, chutesMax) + '%' }" />
                </div>
                <div class="stat-row__value">{{ c.total }}<span class="text-body-small text-medium-emphasis ms-1">({{ share(c.total, chutesTotal) }}%)</span></div>
              </div>
            </div>
          </VCardText>
        </VCard>
      </VCol>
      <VCol v-if="!isSingleDay" cols="12" md="6">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar color="info" variant="tonal"><VIcon icon="tabler-clock-hour-4" /></VAvatar></template>
            <VCardTitle>{{ $t('page.stats.hourly') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.stats.hourlyHint') }}</VCardSubtitle>
          </VCardItem>
          <VCardText class="pt-0"><VChart :option="hourlyOption" autoresize style="height: 300px" /></VCardText>
        </VCard>
      </VCol>
    </VRow>

    <!-- 格口來源 + 狀態分布 + 列印（這台印過才顯示）+ 回報 -->
    <VRow density="compact" class="mt-3">
      <VCol cols="12" sm="6" :lg="data?.printing_used ? 3 : 4">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar color="secondary" variant="tonal"><VIcon icon="tabler-arrows-split-2" /></VAvatar></template>
            <VCardTitle>{{ $t('page.stats.bySource') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.stats.bySourceHint') }}</VCardSubtitle>
          </VCardItem>
          <VDivider />
          <VCardText>
            <div v-if="!sources.length" class="text-body-small text-medium-emphasis">{{ $t('common.noData') }}</div>
            <div v-else class="stat-rows">
              <div v-for="s in sources" :key="s.key" class="stat-row stat-row--chip">
                <VChip size="x-small" :color="sourceMeta(s.key).color" variant="tonal" label>{{ $t(sourceMeta(s.key).key) }}</VChip>
                <div class="stat-row__bar"><div class="stat-row__fill" :class="`stat-row__fill--${sourceMeta(s.key).color}`" :style="{ inlineSize: pct(s.count, sourcesTotal) + '%' }" /></div>
                <div class="stat-row__value">{{ s.count }}<span class="text-body-small text-medium-emphasis ms-1">({{ share(s.count, sourcesTotal) }}%)</span></div>
              </div>
            </div>
          </VCardText>
        </VCard>
      </VCol>
      <VCol cols="12" sm="6" :lg="data?.printing_used ? 3 : 4">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar color="secondary" variant="tonal"><VIcon icon="tabler-list-check" /></VAvatar></template>
            <VCardTitle>{{ $t('page.stats.byStatus') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.stats.byStatusHint') }}</VCardSubtitle>
          </VCardItem>
          <VDivider />
          <VCardText>
            <div v-if="!statuses.length" class="text-body-small text-medium-emphasis">{{ $t('common.noData') }}</div>
            <div v-else class="stat-rows">
              <div v-for="s in statuses" :key="s.status" class="stat-row stat-row--chip">
                <VChip size="x-small" :color="statusMeta(s.status).color" label>{{ $t(statusMeta(s.status).key) }}</VChip>
                <div class="stat-row__bar"><div class="stat-row__fill" :class="`stat-row__fill--${statusMeta(s.status).color}`" :style="{ inlineSize: pct(s.count, statusesTotal) + '%' }" /></div>
                <div class="stat-row__value">{{ s.count }}<span class="text-body-small text-medium-emphasis ms-1">({{ share(s.count, statusesTotal) }}%)</span></div>
              </div>
            </div>
          </VCardText>
        </VCard>
      </VCol>
      <VCol v-if="data?.printing_used" cols="12" sm="6" lg="3">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar :color="print?.failed ? 'error' : 'success'" variant="tonal"><VIcon icon="tabler-printer" /></VAvatar></template>
            <VCardTitle>{{ $t('page.stats.print') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.stats.printHint') }}</VCardSubtitle>
          </VCardItem>
          <VDivider />
          <VCardText>
            <div class="d-flex flex-wrap ga-4 mb-3">
              <div class="text-center"><div class="text-title-large font-weight-bold">{{ print?.total ?? 0 }}</div><div class="text-body-small text-medium-emphasis">{{ $t('page.stats.total') }}</div></div>
              <div class="text-center"><div class="text-title-large font-weight-bold text-success">{{ print?.done ?? 0 }}</div><div class="text-body-small text-medium-emphasis">{{ $t('print.s.done') }}</div></div>
              <div class="text-center"><div class="text-title-large font-weight-bold" :class="print?.failed ? 'text-error' : ''">{{ print?.failed ?? 0 }}</div><div class="text-body-small text-medium-emphasis">{{ $t('print.s.failed') }}</div></div>
              <div class="text-center"><div class="text-title-large font-weight-bold" :class="print?.pending ? 'text-warning' : ''">{{ print?.pending ?? 0 }}</div><div class="text-body-small text-medium-emphasis">{{ $t('print.s.pending') }}</div></div>
              <div class="text-center"><div class="text-title-large font-weight-bold">{{ print?.retried ?? 0 }}</div><div class="text-body-small text-medium-emphasis">{{ $t('page.stats.retried') }}<VTooltip activator="parent" location="bottom">{{ $t('page.stats.printRetriedHint') }}</VTooltip></div></div>
              <div class="text-center"><div class="text-title-large font-weight-bold" :class="printFailRate === null ? 'text-medium-emphasis' : printFailRate > 5 ? 'text-error' : 'text-success'">{{ printFailRate === null ? '—' : printFailRate + '%' }}</div><div class="text-body-small text-medium-emphasis">{{ $t('page.stats.failRate') }}</div></div>
            </div>
            <div v-if="printers.length" class="stat-rows">
              <div v-for="p in printers" :key="p.printer_port" class="stat-row">
                <div class="stat-row__label stat-row__label--wide" :title="p.printer_port">{{ p.chutes.length ? p.chutes.join('/') : p.printer_port }}</div>
                <div class="stat-row__bar stat-row__bar--split">
                  <div class="stat-row__fill stat-row__fill--success" :style="{ inlineSize: pct(p.done, printersMax) + '%' }" />
                  <div class="stat-row__fill stat-row__fill--error" :style="{ inlineSize: pct(p.failed, printersMax) + '%' }" />
                </div>
                <div class="stat-row__value">{{ p.total }}<span v-if="p.failed" class="text-body-small text-error ms-1">({{ p.failed }})</span></div>
              </div>
            </div>
          </VCardText>
        </VCard>
      </VCol>
      <VCol cols="12" sm="6" :lg="data?.printing_used ? 3 : 4">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar :color="report?.failed ? 'error' : 'success'" variant="tonal"><VIcon icon="tabler-cloud-upload" /></VAvatar></template>
            <VCardTitle>{{ $t('page.stats.report') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.stats.reportHint') }}</VCardSubtitle>
          </VCardItem>
          <VDivider />
          <VCardText>
            <div class="d-flex flex-wrap ga-4">
              <div class="text-center"><div class="text-title-large font-weight-bold">{{ report?.total ?? 0 }}</div><div class="text-body-small text-medium-emphasis">{{ $t('page.stats.total') }}</div></div>
              <div class="text-center"><div class="text-title-large font-weight-bold text-success">{{ report?.success ?? 0 }}</div><div class="text-body-small text-medium-emphasis">{{ $t('report.s.success') }}</div></div>
              <div class="text-center"><div class="text-title-large font-weight-bold" :class="report?.failed ? 'text-error' : ''">{{ report?.failed ?? 0 }}</div><div class="text-body-small text-medium-emphasis">{{ $t('report.s.failed') }}</div></div>
              <div class="text-center"><div class="text-title-large font-weight-bold" :class="report?.pending ? 'text-warning' : ''">{{ report?.pending ?? 0 }}</div><div class="text-body-small text-medium-emphasis">{{ $t('report.s.pending') }}</div></div>
              <div class="text-center"><div class="text-title-large font-weight-bold">{{ report?.retried ?? 0 }}</div><div class="text-body-small text-medium-emphasis">{{ $t('page.stats.retried') }}<VTooltip activator="parent" location="bottom">{{ $t('page.stats.reportRetriedHint') }}</VTooltip></div></div>
              <div class="text-center"><div class="text-title-large font-weight-bold" :class="reportFailRate === null ? 'text-medium-emphasis' : reportFailRate > 5 ? 'text-error' : 'text-success'">{{ reportFailRate === null ? '—' : reportFailRate + '%' }}</div><div class="text-body-small text-medium-emphasis">{{ $t('page.stats.failRate') }}</div></div>
            </div>
          </VCardText>
        </VCard>
      </VCol>
    </VRow>

    <!-- 異常原因 + 卡件 -->
    <VRow density="compact" class="mt-3">
      <VCol cols="12" md="6">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar color="error" variant="tonal"><VIcon icon="tabler-alert-triangle" /></VAvatar></template>
            <VCardTitle>{{ $t('page.stats.reasons') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.stats.reasonsHint') }}</VCardSubtitle>
          </VCardItem>
          <VDivider />
          <VCardText>
            <div v-if="!reasons.length" class="empty-state"><VIcon icon="tabler-mood-smile" size="40" class="empty-state__icon" /><div class="empty-state__text">{{ $t('common.noData') }}</div></div>
            <div v-else class="stat-rows">
              <div v-for="r in reasons" :key="r.key" class="stat-row stat-row--reason">
                <div class="stat-row__label stat-row__label--wide" :title="r.key">{{ reasonLabel(r.key) }}</div>
                <div class="stat-row__bar"><div class="stat-row__fill stat-row__fill--error" :style="{ inlineSize: pct(r.count, reasonsTotal) + '%' }" /></div>
                <div class="stat-row__value">{{ r.count }}<span v-if="r.defaulted !== r.count" class="text-body-small text-medium-emphasis ms-1">({{ r.defaulted }})</span></div>
              </div>
            </div>
          </VCardText>
        </VCard>
      </VCol>
      <VCol cols="12" md="6">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar :color="jams?.total ? 'warning' : 'success'" variant="tonal"><VIcon icon="tabler-traffic-cone" /></VAvatar></template>
            <VCardTitle>{{ $t('page.stats.jams') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.stats.jamsHint') }}</VCardSubtitle>
            <template #append>
              <div class="d-flex ga-4 text-center">
                <div><div class="text-title-medium font-weight-bold" :class="jams?.total ? 'text-warning' : ''">{{ jams?.total ?? 0 }}</div><div class="text-body-small text-medium-emphasis">{{ $t('page.stats.jamsTotal') }}</div></div>
                <div><div class="text-title-medium font-weight-bold">{{ jams?.per_thousand ?? 0 }}</div><div class="text-body-small text-medium-emphasis">{{ $t('page.stats.jamsPerThousand') }}</div></div>
              </div>
            </template>
          </VCardItem>
          <VDivider />
          <VCardText>
            <div v-if="!jams?.total" class="empty-state"><VIcon icon="tabler-mood-smile" size="40" class="empty-state__icon" /><div class="empty-state__text">{{ $t('common.noData') }}</div></div>
            <VRow v-else density="compact">
              <VCol cols="12" sm="5">
                <div class="text-body-small text-medium-emphasis mb-2">{{ $t('page.stats.jamsByModule') }}</div>
                <div class="stat-rows">
                  <div v-for="m in jams.by_module" :key="m.key" class="stat-row stat-row--narrow">
                    <div class="stat-row__label">{{ m.key }}</div>
                    <div class="stat-row__bar"><div class="stat-row__fill stat-row__fill--warning" :style="{ inlineSize: pct(m.count, jamModulesMax) + '%' }" /></div>
                    <div class="stat-row__value">{{ m.count }}</div>
                  </div>
                </div>
              </VCol>
              <VCol cols="12" sm="7">
                <div class="text-body-small text-medium-emphasis mb-2">{{ $t('page.stats.jamsByHour') }}</div>
                <VChart :option="jamsHourOption" autoresize style="height: 160px" />
              </VCol>
            </VRow>
          </VCardText>
        </VCard>
      </VCol>
    </VRow>

    <!-- 各段耗時 + 落格口耗時依格口 -->
    <VRow density="compact" class="mt-3">
      <VCol cols="12" md="7">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar color="info" variant="tonal"><VIcon icon="tabler-clock-bolt" /></VAvatar></template>
            <VCardTitle>{{ $t('page.stats.stages') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.stats.stagesHint') }}</VCardSubtitle>
          </VCardItem>
          <VDivider />
          <VTable density="comfortable" class="table-cards">
            <thead><tr>
              <th style="width: 130px;">{{ $t('page.stats.colStage') }}</th><th class="text-end" style="width: 72px;">{{ $t('page.stats.colSamples') }}</th><th class="text-end" style="width: 72px;">p50</th><th class="text-end" style="width: 72px;">p90</th><th class="text-end" style="width: 72px;">p99</th><th class="text-end" style="width: 72px;">{{ $t('page.stats.colMax') }}</th><th class="text-end" style="width: 110px;">{{ $t('page.stats.colOverBudget') }}</th>
            </tr></thead>
            <tbody>
              <tr v-for="st in stages" :key="st.key" :class="{ 'stat-row--zero': !st.samples }">
                <td :data-label="$t('page.stats.colStage')">
                  <div class="font-weight-medium text-no-wrap">{{ $t(`page.stats.stage.${st.key}`) }}</div>
                  <div v-if="st.budget_ms !== null" class="text-body-small text-medium-emphasis text-no-wrap">{{ $t('page.stats.budget') }} {{ fmtDuration(st.budget_ms) }}</div>
                  <VTooltip activator="parent" location="bottom">{{ $t(`page.stats.stageHint.${st.key}`) }}</VTooltip>
                </td>
                <td :data-label="$t('page.stats.colSamples')" class="text-end tabular">{{ st.samples }}</td>
                <td data-label="p50" class="text-end tabular">{{ st.samples ? fmtDuration(st.p50_ms) : '—' }}</td>
                <td data-label="p90" class="text-end tabular" :class="{ 'text-error': st.budget_ms !== null && st.p90_ms > st.budget_ms }">{{ st.samples ? fmtDuration(st.p90_ms) : '—' }}</td>
                <td data-label="p99" class="text-end tabular" :class="{ 'text-error': st.budget_ms !== null && st.p99_ms > st.budget_ms }">{{ st.samples ? fmtDuration(st.p99_ms) : '—' }}</td>
                <td :data-label="$t('page.stats.colMax')" class="text-end tabular">{{ st.samples ? fmtDuration(st.max_ms) : '—' }}</td>
                <td :data-label="$t('page.stats.colOverBudget')" class="text-end tabular">
                  <template v-if="st.budget_ms === null">—</template>
                  <template v-else><span :class="stageOver(st) ? 'text-error font-weight-bold' : ''">{{ st.over_budget }}</span><span class="text-body-small text-medium-emphasis ms-1">({{ share(st.over_budget, st.samples) }}%)</span></template>
                </td>
              </tr>
            </tbody>
          </VTable>
        </VCard>
      </VCol>
      <VCol cols="12" md="5">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar color="info" variant="tonal"><VIcon icon="tabler-route" /></VAvatar></template>
            <VCardTitle>{{ $t('page.stats.travelByChute') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.stats.travelByChuteHint') }}</VCardSubtitle>
          </VCardItem>
          <VDivider />
          <VCardText>
            <div v-if="!travelByChute.length" class="text-body-small text-medium-emphasis">{{ $t('common.noData') }}</div>
            <div v-else class="stat-rows">
              <div v-for="c in travelByChute" :key="c.code" class="stat-row stat-row--travel">
                <div class="stat-row__label stat-row__label--wide">{{ c.code }}</div>
                <div class="stat-row__bar stat-row__bar--split">
                  <div class="stat-row__fill stat-row__fill--info" :style="{ inlineSize: pct(c.p50_ms, travelByChuteMax) + '%' }" />
                  <div class="stat-row__fill stat-row__fill--info-light" :style="{ inlineSize: pct(c.p90_ms - c.p50_ms, travelByChuteMax) + '%' }" />
                </div>
                <div class="stat-row__value">{{ fmtDuration(c.p50_ms) }}<span class="text-body-small text-medium-emphasis ms-1">/ {{ fmtDuration(c.p90_ms) }}</span></div>
              </div>
            </div>
          </VCardText>
        </VCard>
      </VCol>
    </VRow>

    <!-- 裝置穩定性 + 重複進線 -->
    <VRow density="compact" class="mt-3">
      <VCol cols="12" md="6">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar :color="devices.some(d => d.disconnects) ? 'warning' : 'success'" variant="tonal"><VIcon icon="tabler-plug-connected" /></VAvatar></template>
            <VCardTitle>{{ $t('page.stats.devices') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.stats.devicesHint') }}</VCardSubtitle>
          </VCardItem>
          <VDivider />
          <VTable density="comfortable" class="table-cards">
            <thead><tr>
              <th></th><th class="text-end">{{ $t('page.stats.disconnects') }}</th><th class="text-end">{{ $t('page.stats.longestOutage') }}</th><th class="text-end">{{ $t('page.stats.totalOutage') }}</th><th class="text-end">{{ $t('page.stats.lastDisconnect') }}</th>
            </tr></thead>
            <tbody>
              <tr v-for="d in devices" :key="d.device">
                <td :data-label="$t('page.stats.colDevice')"><VIcon :icon="deviceIcon[d.device]" size="16" class="me-1" :color="d.disconnects ? 'warning' : 'success'" />{{ $t('device.' + d.device) }}</td>
                <td :data-label="$t('page.stats.disconnects')" class="text-end tabular" :class="d.disconnects ? 'text-warning font-weight-bold' : ''">{{ d.disconnects }}</td>
                <td :data-label="$t('page.stats.longestOutage')" class="text-end tabular">{{ fmtOutage(d.longest_ms) }}</td>
                <td :data-label="$t('page.stats.totalOutage')" class="text-end tabular">{{ fmtOutage(d.total_ms) }}</td>
                <td :data-label="$t('page.stats.lastDisconnect')" class="text-end text-no-wrap">{{ d.last_disconnect_at ? d.last_disconnect_at.slice(5, 16) : $t('page.stats.neverDisconnected') }}</td>
              </tr>
            </tbody>
          </VTable>
          <VCardText v-if="devices.some(d => d.disconnects)" class="pt-2">
            <div class="text-body-small text-medium-emphasis mb-1">{{ $t('page.stats.devicesByHour') }}</div>
            <VChart :option="devicesHourOption" autoresize style="height: 170px" />
          </VCardText>
        </VCard>
      </VCol>
      <VCol cols="12" md="6">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar :color="duplicates?.barcodes ? 'warning' : 'success'" variant="tonal"><VIcon icon="tabler-copy" /></VAvatar></template>
            <VCardTitle>{{ $t('page.stats.duplicates') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.stats.duplicatesHint') }}</VCardSubtitle>
          </VCardItem>
          <VDivider />
          <VCardText>
            <div class="d-flex flex-wrap ga-4 mb-3">
              <div class="text-center"><div class="text-title-large font-weight-bold" :class="duplicates?.barcodes ? 'text-warning' : ''">{{ duplicates?.barcodes ?? 0 }}</div><div class="text-body-small text-medium-emphasis">{{ $t('page.stats.dupBarcodes') }}</div></div>
              <div class="text-center"><div class="text-title-large font-weight-bold">{{ duplicates?.extra_runs ?? 0 }}</div><div class="text-body-small text-medium-emphasis">{{ $t('page.stats.dupExtraRuns') }}</div></div>
              <div class="text-center"><div class="text-title-large font-weight-bold">{{ duplicates?.twice ?? 0 }}</div><div class="text-body-small text-medium-emphasis">{{ $t('page.stats.dupTwice') }}</div></div>
              <div class="text-center"><div class="text-title-large font-weight-bold">{{ duplicates?.thrice ?? 0 }}</div><div class="text-body-small text-medium-emphasis">{{ $t('page.stats.dupThrice') }}</div></div>
              <div class="text-center"><div class="text-title-large font-weight-bold">{{ duplicates?.more ?? 0 }}</div><div class="text-body-small text-medium-emphasis">{{ $t('page.stats.dupMore') }}</div></div>
            </div>
            <div v-if="!duplicates?.top?.length" class="text-body-small text-medium-emphasis">{{ $t('page.stats.noDuplicates') }}</div>
            <template v-else>
              <div class="text-body-small text-medium-emphasis mb-1">{{ $t('page.stats.dupTop') }}</div>
              <VTable density="compact" class="dup-table">
                <thead><tr><th>{{ $t('page.stats.colBarcode') }}</th><th class="text-end">{{ $t('page.stats.colRuns') }}</th><th>{{ $t('page.stats.colChutes') }}</th></tr></thead>
                <tbody>
                  <tr v-for="r in duplicates.top" :key="r.barcode">
                    <td class="selectable text-no-wrap">{{ r.barcode }}</td>
                    <td class="text-end tabular">{{ r.count }}</td>
                    <td class="text-medium-emphasis">{{ r.chutes }}</td>
                  </tr>
                </tbody>
              </VTable>
            </template>
          </VCardText>
        </VCard>
      </VCol>
    </VRow>

    <!-- 讀碼失敗率趨勢 + 小車異常率 -->
    <VRow density="compact" class="mt-3">
      <VCol cols="12" md="6">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar color="warning" variant="tonal"><VIcon icon="tabler-barcode-off" /></VAvatar></template>
            <VCardTitle>{{ $t('page.stats.noreadRate') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.stats.noreadRateHint') }}</VCardSubtitle>
          </VCardItem>
          <VCardText class="pt-0"><VChart :option="noreadOption" autoresize style="height: 240px" /></VCardText>
        </VCard>
      </VCol>
      <VCol cols="12" md="6">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar color="warning" variant="tonal"><VIcon icon="tabler-ruler-measure" /></VAvatar></template>
            <VCardTitle>{{ $t('page.stats.noreadByLength') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.stats.noreadByLengthHint') }}</VCardSubtitle>
          </VCardItem>
          <VCardText class="pt-0"><VChart :option="noreadLengthOption" autoresize style="height: 240px" /></VCardText>
        </VCard>
      </VCol>
      <VCol cols="12" md="6">
        <VCard class="card-shadow h-100">
          <VCardItem>
            <template #prepend><VAvatar :color="carts.some(cartHot) ? 'error' : 'info'" variant="tonal"><VIcon icon="tabler-truck-loading" /></VAvatar></template>
            <VCardTitle>{{ $t('page.stats.byCart') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.stats.byCartHint') }}</VCardSubtitle>
          </VCardItem>
          <VCardText class="pt-0">
            <div v-if="!carts.length" class="empty-state"><VIcon icon="tabler-truck-off" size="40" class="empty-state__icon" /><div class="empty-state__text">{{ $t('common.noData') }}</div></div>
            <VChart v-else :option="cartOption" autoresize style="height: 240px" />
          </VCardText>
        </VCard>
      </VCol>
    </VRow>

    <!-- 星期 × 小時熱力圖 -->
    <VRow density="compact" class="mt-3">
      <VCol cols="12">
        <VCard class="card-shadow">
          <VCardItem>
            <template #prepend><VAvatar color="primary" variant="tonal"><VIcon icon="tabler-temperature" /></VAvatar></template>
            <VCardTitle>{{ $t('page.stats.heatmapTitle') }}</VCardTitle>
            <VCardSubtitle>{{ $t('page.stats.heatmapHint') }}</VCardSubtitle>
          </VCardItem>
          <VDivider />
          <VCardText>
            <div v-if="!heatmapCells.length" class="empty-state"><VIcon icon="tabler-clock-off" size="40" class="empty-state__icon" /><div class="empty-state__text">{{ $t('common.noData') }}</div></div>
            <VChart v-else :option="heatmapOption" autoresize style="height: 280px" />
          </VCardText>
        </VCard>
      </VCol>
    </VRow>
  </div>
</template>

<style lang="scss" scoped>
// 日期區間的兩個欄位：桌面固定寬度，手機各佔一半（與 LabelPrint 印單統計頁同一套修法）
.stats-date-field { inline-size: 170px; }
@media (max-width: 639.98px) {
  .stats-date-row { flex: 1 1 100%; }
  .stats-date-field {
    flex: 1 1 0;
    inline-size: auto;
    min-inline-size: 0;
    :deep(.v-field__input) { padding-inline-end: 4px; }
  }
}
.range-rate { border-inline-start: 1px solid rgba(var(--v-theme-on-surface), 0.12); padding-inline-start: 16px; }
// KPI 卡：今日為主視覺（主色描邊）；字級寫在這裡，Vuetify 的字級 class 鎖在 !important 分層改不動
.kpi-card {
  &--primary { border-block-start: 3px solid rgb(var(--v-theme-primary)); }
  &__count { font-size: 2rem; line-height: 2.25rem; white-space: nowrap; }
  @media (max-width: 639.98px) {
    &__count { font-size: 1.5rem; line-height: 1.75rem; }
  }
}
.stat-rows { display: flex; flex-direction: column; gap: 8px; }
.stat-row {
  display: grid;
  grid-template-columns: 96px 1fr 88px;
  align-items: center;
  gap: 8px;
  &--zero { opacity: 0.45; }
  &--chip { grid-template-columns: 96px 1fr 72px; }
  &--reason { grid-template-columns: 150px 1fr 88px; }
  &--narrow { grid-template-columns: 32px 1fr 40px; }
  &--travel { grid-template-columns: 40px 1fr 110px; }
}
.stat-row__label {
  font-size: 12px;
  color: rgba(var(--v-theme-on-surface), 0.7);
  &--wide { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
}
.stat-row__bar {
  block-size: 10px;
  background: rgba(var(--v-theme-on-surface), 0.05);
  border-radius: 5px;
  overflow: hidden;
  // 完成／異常兩段接在同一條上
  &--split { display: flex; }
}
.stat-row__fill {
  block-size: 100%;
  background: rgb(var(--v-theme-primary));
  transition: inline-size 0.3s ease;
  &--info { background: rgb(var(--v-theme-info)); }
  &--success { background: rgb(var(--v-theme-success)); }
  &--warning { background: rgb(var(--v-theme-warning)); }
  &--error { background: rgb(var(--v-theme-error)); }
  &--secondary { background: rgb(var(--v-theme-secondary)); }
  &--info-light { background: rgba(var(--v-theme-info), 0.35); }
}
.tabular { font-variant-numeric: tabular-nums; white-space: nowrap; }
.dup-table :deep(td), .dup-table :deep(th) { font-size: 12px; }
.stat-row__value { font-size: 13px; font-weight: 600; text-align: end; font-variant-numeric: tabular-nums; white-space: nowrap; }
.empty-state { display: flex; flex-direction: column; align-items: center; justify-content: center; padding: 32px 16px; gap: 8px; }
.empty-state__icon { color: rgba(var(--v-theme-on-surface), 0.25); }
.compare-row {
  display: grid;
  grid-template-columns: 72px 1fr auto;
  align-items: center;
  gap: 12px;
  padding-block: 8px;
  & + & { border-block-start: 1px dashed rgba(var(--v-theme-on-surface), 0.08); }
}
.compare-row__label { font-size: 13px; color: rgba(var(--v-theme-on-surface), 0.7); }
.compare-row__delta { display: flex; align-items: center; }
</style>
