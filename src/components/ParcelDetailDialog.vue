<script setup>
/** 單件包裹：訊號時間軸（相對 ~P 的毫秒）與列印任務 */
import { api } from '@/api/http'
import { fmtMs, fmtDuration, statusMeta, sourceMeta } from '@/composables/useFormat'
import { toast } from 'vue3-toastify'
import { useI18n } from 'vue-i18n'

const props = defineProps({ modelValue: Boolean, parcelId: { type: Number, default: null } })
const emit = defineEmits(['update:modelValue'])

const { t } = useI18n()
const loading = ref(false)
const data = ref(null)

watch(() => [props.modelValue, props.parcelId], async ([open, id]) => {
  if (!open || !id) return
  loading.value = true
  data.value = null
  try {
    data.value = await api.parcel(id)
  } catch (e) {
    toast(e.message, { type: 'error' })
  } finally {
    loading.value = false
  }
}, { immediate: true })

const rel = ts => data.value ? ts - data.value.parcel.started_ms : 0
const sourceColor = { belt: 'primary', sorter: 'info', camera: 'secondary', api: 'success', tracker: 'warning', printer: 'secondary' }

// 一眼看出卡在哪：超過門檻的區段標紅（門檻取 protocol-spec 的停線規則與實測 p95）
const LIMITS = { knToC: 300, jToG: 550, pToO: 2000 }
const anomalies = computed(() => {
  const out = new Map()
  if (!data.value) return out
  const ev = data.value.events
  const first = kind => ev.find(e => e.kind === kind)
  const mark = (e, text) => { if (e) out.set(e, [...(out.get(e) || []), text]) }
  const pair = (fromKind, toKind, limit, key) => {
    const a = first(fromKind), b = first(toKind)
    if (a && b && b.ts_ms - a.ts_ms > limit) mark(b, t(key, { ms: b.ts_ms - a.ts_ms, limit }))
  }
  pair('Kn', 'c', LIMITS.knToC, 'parcel.slowKnC')
  pair('j', 'g', LIMITS.jToG, 'parcel.slowJG')
  pair('P', 'O', LIMITS.pToO, 'parcel.slowPO')
  for (const e of ev) if (e.kind === 'chute_late') mark(e, t('parcel.chuteLate'))
  return out
})

// 訊號翻成現場看得懂的話；沒對照到的照原代碼顯示
const kindLabel = e => {
  const key = `parcel.kind.${e.kind}`
  const s = t(key)
  return s === key ? e.kind : s
}

// 關鍵里程碑：上線 → 讀碼 → 取得格口 → 交接 → 上車 → 落格。每一步顯示「距上一步幾毫秒」，
// 沒走到的步驟顯示空白，一眼看出停在哪
const MILESTONES = [
  { key: 'P', label: 'parcel.step.P' },
  { key: 'bind', label: 'parcel.step.bind' },
  { key: ['chute', 'chute_default'], label: 'parcel.step.chute' },
  { key: 'O', label: 'parcel.step.O' },
  { key: 'j', label: 'parcel.step.j' },
  { key: ['e', 'k', 'u'], label: 'parcel.step.e' },
]
const milestones = computed(() => {
  if (!data.value) return []
  const ev = data.value.events
  let prev = null
  return MILESTONES.map(m => {
    const keys = Array.isArray(m.key) ? m.key : [m.key]
    const hit = ev.find(e => keys.includes(e.kind))
    const row = { label: t(m.label), at: hit ? fmtMs(hit.ts_ms).slice(6) : '', delta: hit && prev ? hit.ts_ms - prev.ts_ms : null, done: !!hit, kind: hit?.kind }
    if (hit) prev = hit
    return row
  })
})
</script>

<template>
  <VDialog :model-value="modelValue" max-width="860" scrollable @update:model-value="emit('update:modelValue', $event)">
    <VCard>
      <VCardTitle class="d-flex align-center">
        <span>{{ $t('parcel.detail') }}</span>
        <VSpacer />
        <VBtn icon variant="text" @click="emit('update:modelValue', false)"><VIcon icon="tabler-x" /></VBtn>
      </VCardTitle>
      <VCardText>
        <VProgressLinear v-if="loading" indeterminate />
        <template v-else-if="data">
          <!-- 主要資訊：條碼最大，其餘一列 chip 與一列小欄，時間不再被擠成兩行 -->
          <div class="d-flex align-center flex-wrap ga-2 mb-2">
            <span class="text-h5 font-weight-bold selectable">{{ data.parcel.barcode }}</span>
            <VChip size="small" label color="primary" variant="tonal" class="font-weight-bold">{{ $t('parcel.chute') }} {{ data.parcel.chute_code || '—' }}</VChip>
            <VChip size="small" label :color="sourceMeta(data.parcel.chute_source).color" variant="tonal">{{ $t(sourceMeta(data.parcel.chute_source).key) }}</VChip>
            <VChip size="small" label :color="statusMeta(data.parcel.status).color">{{ $t(statusMeta(data.parcel.status).key) }}</VChip>
          </div>
          <div class="detail-facts mb-4">
            <div><span class="text-caption text-medium-emphasis">{{ $t('parcel.startedAt') }}</span><span class="text-no-wrap">{{ data.parcel.started_at }}</span></div>
            <div><span class="text-caption text-medium-emphasis">{{ $t('parcel.travel') }}</span><span>{{ fmtDuration(data.parcel.travel_ms) }}</span></div>
            <div><span class="text-caption text-medium-emphasis">{{ $t('parcel.cart') }}</span><span>{{ data.parcel.cart ?? '—' }}</span></div>
            <div><span class="text-caption text-medium-emphasis">{{ $t('parcel.slot') }}</span><span>{{ data.parcel.belt_slot ?? '—' }}</span></div>
            <div><span class="text-caption text-medium-emphasis">{{ $t('parcel.responseId') }}</span><span>{{ data.parcel.response_id ?? '—' }}</span></div>
          </div>

          <!-- 里程碑：一眼看這件走到哪、每段花多久 -->
          <h4 class="mb-2">{{ $t('parcel.steps') }}</h4>
          <div class="steps mb-4">
            <div v-for="(m, i) in milestones" :key="i" class="step" :class="{ 'step--done': m.done, 'step--bad': m.done && ['k', 'u'].includes(m.kind) }">
              <div class="step__dot"><VIcon :icon="m.done ? (['k', 'u'].includes(m.kind) ? 'tabler-alert-triangle' : 'tabler-check') : 'tabler-minus'" size="12" /></div>
              <div class="step__label">{{ m.done && m.kind === 'k' ? $t('parcel.kind.k') : m.done && m.kind === 'u' ? $t('parcel.kind.u') : m.label }}</div>
              <div class="step__time text-body-small text-medium-emphasis">{{ m.at || '—' }}</div>
              <div v-if="m.delta != null" class="step__delta text-body-small">+{{ m.delta }} ms</div>
            </div>
          </div>

          <h4 class="mb-2">{{ $t('parcel.timeline') }}</h4>
          <VTable density="compact" class="mb-4 timeline-table">
            <thead><tr><th>{{ $t('parcel.time') }}</th><th class="text-end">+ms</th><th>{{ $t('parcel.signal') }}</th><th>{{ $t('parcel.raw') }}</th></tr></thead>
            <tbody>
              <tr v-for="(e, i) in data.events" :key="i" :class="{ 'row-anomaly': anomalies.has(e) }">
                <td class="text-no-wrap">{{ fmtMs(e.ts_ms).slice(6) }}</td>
                <td class="text-end text-no-wrap" :class="{ 'text-error font-weight-bold': anomalies.has(e) }">{{ rel(e.ts_ms) }}</td>
                <td class="text-no-wrap">
                  <VChip size="x-small" :color="sourceColor[e.source] || 'secondary'" variant="tonal" class="me-2">{{ $t(`parcel.src.${e.source}`) }}</VChip>
                  <span class="font-weight-medium">{{ kindLabel(e) }}</span>
                  <VChip v-for="(a, k) in anomalies.get(e) || []" :key="k" size="x-small" color="error" variant="tonal" label class="ms-2">
                    <VIcon icon="tabler-alert-triangle" size="12" start />{{ a }}
                  </VChip>
                </td>
                <td class="selectable text-medium-emphasis"><code class="text-body-small">{{ e.raw }}</code></td>
              </tr>
            </tbody>
          </VTable>

          <template v-if="data.print_jobs.length">
            <h4 class="mb-2">{{ $t('parcel.printJobs') }}</h4>
            <VTable density="compact">
              <thead><tr><th>{{ $t('print.printer') }}</th><th>{{ $t('print.status') }}</th><th>{{ $t('print.attempts') }}</th><th>{{ $t('print.error') }}</th></tr></thead>
              <tbody>
                <tr v-for="j in data.print_jobs" :key="j.id">
                  <td>{{ j.chute_code }} / {{ j.printer_port }}</td><td>{{ $t(`print.s.${j.status}`) }}</td><td>{{ j.attempts }}</td><td class="text-error">{{ j.last_error }}</td>
                </tr>
              </tbody>
            </VTable>
          </template>
        </template>
      </VCardText>
    </VCard>
  </VDialog>

</template>

<style scoped>
.row-anomaly { background: rgba(var(--v-theme-error), 0.06); }
.detail-facts { display: flex; flex-wrap: wrap; gap: 8px 28px; }
.detail-facts > div { display: flex; flex-direction: column; }
.steps { display: grid; grid-template-columns: repeat(6, 1fr); gap: 6px; }
.step { position: relative; padding: 8px 6px 6px; border-radius: 8px; border: 1px solid rgba(var(--v-border-color), var(--v-border-opacity)); text-align: center; opacity: 0.55; }
.step--done { opacity: 1; border-color: rgb(var(--v-theme-success)); background: rgba(var(--v-theme-success), 0.06); }
.step--bad { border-color: rgb(var(--v-theme-error)); background: rgba(var(--v-theme-error), 0.08); }
.step__dot { inline-size: 20px; block-size: 20px; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center; background: rgba(var(--v-theme-on-surface), 0.08); margin-block-end: 4px; }
.step--done .step__dot { background: rgb(var(--v-theme-success)); color: #fff; }
.step--bad .step__dot { background: rgb(var(--v-theme-error)); }
.step__label { font-weight: 600; font-size: 0.85rem; }
.step__delta { color: rgb(var(--v-theme-primary)); }
.timeline-table code { white-space: pre-wrap; word-break: break-all; }
@media (max-width: 639px) { .steps { grid-template-columns: repeat(3, 1fr); } }
</style>
