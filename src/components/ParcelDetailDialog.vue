<script setup>
/** 單件包裹：訊號時間軸（相對 ~P 的毫秒）與列印任務 */
import { api, parcelImageUrl } from '@/api/http'
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
// 讀碼站照片：點縮圖放大看；讀碼失敗件另有原圖可開
const viewImage = ref(null)
const fmtKb = n => `${Math.round(n / 1024)} KB`
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
          <VRow dense class="mb-3">
            <VCol cols="12" sm="4"><div class="text-caption">{{ $t('parcel.barcode') }}</div><div class="text-h6 selectable">{{ data.parcel.barcode }}</div></VCol>
            <VCol cols="6" sm="2"><div class="text-caption">{{ $t('parcel.chute') }}</div><div class="text-h6">{{ data.parcel.chute_code || '—' }}</div></VCol>
            <VCol cols="6" sm="3"><div class="text-caption">{{ $t('parcel.source') }}</div><VChip size="small" :color="sourceMeta(data.parcel.chute_source).color">{{ $t(sourceMeta(data.parcel.chute_source).key) }}</VChip></VCol>
            <VCol cols="6" sm="3"><div class="text-caption">{{ $t('parcel.status') }}</div><VChip size="small" :color="statusMeta(data.parcel.status).color">{{ $t(statusMeta(data.parcel.status).key) }}</VChip></VCol>
            <VCol cols="6" sm="3"><div class="text-caption">{{ $t('parcel.startedAt') }}</div><div>{{ data.parcel.started_at }}</div></VCol>
            <VCol cols="6" sm="3"><div class="text-caption">{{ $t('parcel.travel') }}</div><div>{{ fmtDuration(data.parcel.travel_ms) }}</div></VCol>
            <VCol cols="6" sm="2"><div class="text-caption">{{ $t('parcel.cart') }}</div><div>{{ data.parcel.cart ?? '—' }}</div></VCol>
            <VCol cols="6" sm="2"><div class="text-caption">{{ $t('parcel.slot') }}</div><div>{{ data.parcel.belt_slot ?? '—' }}</div></VCol>
            <VCol cols="6" sm="2"><div class="text-caption">{{ $t('parcel.responseId') }}</div><div>{{ data.parcel.response_id ?? '—' }}</div></VCol>
          </VRow>

          <template v-if="data.images?.length">
            <h4 class="mb-2">{{ $t('parcel.images') }}</h4>
            <div class="d-flex flex-wrap ga-3 mb-4">
              <div v-for="img in data.images" :key="img.id" class="parcel-image">
                <img :src="parcelImageUrl(img.id)" :alt="img.file_name" class="parcel-image__thumb" @click="viewImage = img">
                <div class="text-body-small text-medium-emphasis mt-1">{{ img.received_at.slice(11, 19) }} · {{ fmtKb(img.size) }}</div>
                <a v-if="img.has_orig" :href="parcelImageUrl(img.id, true)" target="_blank" rel="noopener" class="text-body-small">{{ $t('parcel.imageOriginal') }}</a>
              </div>
            </div>
          </template>
          <div v-else class="text-body-small text-medium-emphasis mb-4">{{ $t('parcel.noImage') }}</div>

          <h4 class="mb-2">{{ $t('parcel.timeline') }}</h4>
          <VTable density="compact" class="mb-4">
            <thead><tr><th>{{ $t('parcel.time') }}</th><th>+ms</th><th>{{ $t('parcel.sourceCol') }}</th><th>{{ $t('parcel.signal') }}</th><th>{{ $t('parcel.raw') }}</th></tr></thead>
            <tbody>
              <tr v-for="(e, i) in data.events" :key="i" :class="{ 'row-anomaly': anomalies.has(e) }">
                <td class="text-no-wrap">{{ fmtMs(e.ts_ms) }}</td>
                <td class="text-end" :class="{ 'text-error font-weight-bold': anomalies.has(e) }">{{ rel(e.ts_ms) }}</td>
                <td><VChip size="x-small" :color="sourceColor[e.source] || 'secondary'" variant="tonal">{{ e.source }}</VChip></td>
                <td class="font-weight-medium">{{ e.kind }}</td>
                <td class="selectable text-medium-emphasis">
                  <code>{{ e.raw }}</code>
                  <VChip v-for="(a, k) in anomalies.get(e) || []" :key="k" size="x-small" color="error" variant="tonal" label class="ms-2">
                    <VIcon icon="tabler-alert-triangle" size="12" start />{{ a }}
                  </VChip>
                </td>
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

  <VDialog :model-value="!!viewImage" max-width="1400" @update:model-value="v => { if (!v) viewImage = null }">
    <VCard v-if="viewImage">
      <VCardTitle class="d-flex align-center text-body-large">
        <span class="selectable">{{ viewImage.file_name }}</span>
        <VSpacer />
        <a v-if="viewImage.has_orig" :href="parcelImageUrl(viewImage.id, true)" target="_blank" rel="noopener" class="me-3 text-body-medium">{{ $t('parcel.imageOriginal') }}</a>
        <VBtn icon variant="text" @click="viewImage = null"><VIcon icon="tabler-x" /></VBtn>
      </VCardTitle>
      <VCardText class="pa-0"><img :src="parcelImageUrl(viewImage.id)" :alt="viewImage.file_name" class="parcel-image__full"></VCardText>
    </VCard>
  </VDialog>
</template>

<style scoped>
.row-anomaly { background: rgba(var(--v-theme-error), 0.06); }
.parcel-image { inline-size: 160px; }
.parcel-image__thumb { inline-size: 160px; block-size: 120px; object-fit: cover; border-radius: 6px; cursor: zoom-in; border: 1px solid rgba(var(--v-border-color), var(--v-border-opacity)); }
.parcel-image__full { display: block; inline-size: 100%; max-block-size: 85vh; object-fit: contain; background: #111; }
</style>
