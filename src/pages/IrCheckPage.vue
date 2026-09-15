<script setup>
/**
 * 光電檢查：每台分揀機的偵測光電有沒有被遮蔽；點一台看每顆光電的即時狀態，壞掉的可先整台屏蔽讓線能跑。
 * 對應舊系統 ir.html／ir.go（Kd[ 總覽、p1 單台、KY 屏蔽）。
 */
import { useI18n } from 'vue-i18n'
import { api } from '@/api/http'
import { useSettingsPassword } from '@/composables/useSettingsPassword'
import AppHeader from '@/components/AppHeader.vue'
import PageActions from '@/components/PageActions.vue'
import { toast } from 'vue3-toastify'

const { t } = useI18n()
const pw = useSettingsPassword()

const devices = ref([]) // [{ m2, status }]
const checking = ref(false)
const checkedAt = ref('')
const errorMsg = ref('')

const check = async () => {
  checking.value = true
  errorMsg.value = ''
  try {
    const d = await api.irStatus()
    devices.value = d.devices
    checkedAt.value = new Date().toLocaleTimeString('zh-Hant', { hour12: false })
  } catch (e) {
    errorMsg.value = e.message
  } finally {
    checking.value = false
  }
}
onMounted(check)

const statusMeta = { 0: { color: 'secondary', key: 'page.ir.s.unknown' }, 1: { color: 'success', key: 'page.ir.s.ok' }, 2: { color: 'error', key: 'page.ir.s.blocked' } }
const actions = computed(() => [{ key: 'check', label: t('page.ir.check'), icon: 'tabler-refresh', loading: checking.value, onClick: check }])

// ---- 單台詳情 ----
const detail = ref({ open: false, m2: null, status: 0, irNum: 0, triggered: [], loading: false, error: '' })
const openDetail = async dev => {
  if (dev.status === 0) { toast(t('page.ir.checkFirst'), { type: 'warning' }); return }
  detail.value = { open: true, m2: dev.m2, status: dev.status, irNum: 0, triggered: [], loading: false, error: '' }
  await loadDetail()
}
const loadDetail = async () => {
  detail.value.loading = true
  detail.value.error = ''
  try {
    const d = await api.irDetail(detail.value.m2)
    detail.value.irNum = d.ir_num
    detail.value.triggered = d.triggered
  } catch (e) {
    detail.value.error = e.message
  } finally {
    detail.value.loading = false
  }
}

// 光電在小車四周的實際排列（沿用舊頁面的對照，0 = 空格）；台數不同的分揀機顆數不同
const LAYOUTS = {
  38: [[0, 1, 2, 3, 4, 5, 0], [18, 19, 26, 27, 34, 35, 6], [17, 20, 25, 28, 33, 36, 7], [16, 21, 24, 29, 32, 37, 8], [15, 22, 23, 30, 31, 38, 9], [0, 14, 13, 12, 11, 10, 0]],
  45: [[0, 1, 2, 3, 4, 5, 0], [20, 21, 30, 31, 40, 41, 6], [19, 22, 29, 32, 39, 42, 7], [18, 23, 28, 33, 38, 43, 8], [17, 24, 27, 34, 37, 44, 9], [16, 25, 26, 35, 36, 45, 10], [0, 15, 14, 13, 12, 11, 0]],
  26: [[0, 1, 2, 3, 4, 5, 0], [18, 0, 19, 0, 26, 0, 6], [17, 0, 20, 0, 25, 0, 7], [16, 0, 21, 0, 24, 0, 8], [15, 0, 22, 0, 23, 0, 9], [0, 14, 13, 12, 11, 10, 0]],
}
const layout = computed(() => {
  if (LAYOUTS[detail.value.irNum]) return LAYOUTS[detail.value.irNum]
  // 沒有對照表的顆數：每列 7 顆依序排
  const n = detail.value.irNum
  const rows = []
  for (let i = 1; i <= n; i += 7) rows.push(Array.from({ length: 7 }, (_, k) => (i + k <= n ? i + k : 0)))
  return rows
})
// 每顆的狀態：'on' 被遮蔽（有東西擋著或光電壞了）、'off' 通、'na' 沒讀到
const cellState = idx => {
  if (!idx) return 'empty'
  const v = detail.value.triggered[idx - 1]
  return v == null ? 'na' : v ? 'on' : 'off'
}

const blocking = ref(false)
const setBlock = async block => {
  if (!(await pw.ensure())) return
  blocking.value = true
  try {
    await api.irBlock(detail.value.m2, block)
    toast(t(block ? 'page.ir.blocked' : 'page.ir.unblocked'), { type: 'success' })
  } catch (e) {
    if (e.status === 403) pw.forget()
    toast(e.message, { type: 'error' })
  } finally {
    blocking.value = false
  }
}
</script>

<template>
  <div>
    <AppHeader :title="$t('page.ir.title')" :subtitle="$t('page.ir.subtitle')" :subtitle-short="$t('page.ir.subtitleShort')" icon="tabler-viewfinder">
      <template #actions><PageActions :items="actions" /></template>
    </AppHeader>

    <VAlert type="info" variant="tonal" density="compact" class="mb-3" icon="tabler-info-circle">
      <div class="text-body-medium">{{ $t('page.ir.hint1') }}</div>
      <div class="text-body-medium mt-1">{{ $t('page.ir.hint2') }}</div>
    </VAlert>

    <VAlert v-if="errorMsg" type="error" variant="tonal" density="compact" class="mb-3">{{ errorMsg }}</VAlert>

    <VCard class="card-shadow">
      <VCardItem>
        <template #prepend><VAvatar color="primary" variant="tonal"><VIcon icon="tabler-route" /></VAvatar></template>
        <VCardTitle>{{ $t('page.ir.devices') }}</VCardTitle>
        <VCardSubtitle v-if="checkedAt">{{ $t('page.ir.checkedAt', { time: checkedAt }) }}</VCardSubtitle>
        <VCardSubtitle v-else>{{ $t('page.ir.notChecked') }}</VCardSubtitle>
      </VCardItem>
      <VCardText>
        <div v-if="!devices.length && !checking" class="py-6 d-flex align-center justify-center text-medium-emphasis">
          <VIcon icon="tabler-alert-circle" size="20" class="me-1" />{{ $t('page.ir.empty') }}
        </div>
        <div class="ir-grid">
          <VCard v-for="d in devices" :key="d.m2" class="ir-device" :class="`ir-device--${d.status}`" variant="tonal" :color="statusMeta[d.status].color" hover @click="openDetail(d)">
            <div class="ir-device__no">#{{ d.m2 + 1 }}</div>
            <div class="ir-device__label">{{ $t(statusMeta[d.status].key) }}</div>
          </VCard>
        </div>
        <div class="d-flex flex-wrap ga-4 mt-4 text-body-small text-medium-emphasis">
          <span><VIcon icon="tabler-square-filled" color="success" size="14" class="me-1" />{{ $t('page.ir.legend.ok') }}</span>
          <span><VIcon icon="tabler-square-filled" color="error" size="14" class="me-1" />{{ $t('page.ir.legend.blocked') }}</span>
          <span><VIcon icon="tabler-square-filled" color="secondary" size="14" class="me-1" />{{ $t('page.ir.legend.unknown') }}</span>
        </div>
      </VCardText>
    </VCard>

    <VDialog v-model="detail.open" max-width="560">
      <VCard>
        <VCardTitle class="d-flex align-center">
          <span>{{ $t('page.ir.detailTitle', { n: (detail.m2 ?? 0) + 1 }) }}</span>
          <VChip size="small" :color="statusMeta[detail.status].color" variant="tonal" label class="ms-3">{{ $t(statusMeta[detail.status].key) }}</VChip>
          <VSpacer />
          <VBtn icon variant="text" @click="detail.open = false"><VIcon icon="tabler-x" /></VBtn>
        </VCardTitle>
        <VCardText>
          <VProgressLinear v-if="detail.loading" indeterminate class="mb-3" />
          <VAlert v-if="detail.error" type="error" variant="tonal" density="compact" class="mb-3">{{ detail.error }}</VAlert>
          <template v-if="detail.irNum">
            <div class="text-body-small text-medium-emphasis mb-2">{{ $t('page.ir.detailHint') }}</div>
            <div class="ir-ring" :class="{ 'ir-ring--abnormal': detail.status === 2 }">
              <div v-for="(row, r) in layout" :key="r" class="ir-ring__row">
                <div v-for="(idx, c) in row" :key="c" class="ir-cell" :class="`ir-cell--${cellState(idx)}`">
                  <span v-if="idx">{{ idx }}</span>
                </div>
              </div>
            </div>
            <div class="d-flex flex-wrap ga-4 mt-3 text-body-small text-medium-emphasis" :class="{ 'ir-ring--abnormal': detail.status === 2 }">
              <span><span class="ir-cell ir-cell--legend ir-cell--off" /> {{ $t('page.ir.legend.clear') }}</span>
              <span><span class="ir-cell ir-cell--legend ir-cell--on" /> {{ $t('page.ir.legend.triggered') }}</span>
              <span><span class="ir-cell ir-cell--legend ir-cell--na" /> {{ $t('page.ir.legend.noData') }}</span>
            </div>
          </template>
        </VCardText>
        <VCardActions class="px-4 pb-4 flex-wrap ga-2">
          <VBtn variant="outlined" :loading="detail.loading" @click="loadDetail"><VIcon icon="tabler-refresh" size="16" class="me-1" />{{ $t('common.reload') }}</VBtn>
          <VSpacer />
          <VBtn color="error" variant="tonal" :loading="blocking" @click="setBlock(true)"><VIcon icon="tabler-eye-off" size="16" class="me-1" />{{ $t('page.ir.block') }}</VBtn>
          <VBtn color="success" variant="tonal" :loading="blocking" @click="setBlock(false)"><VIcon icon="tabler-eye" size="16" class="me-1" />{{ $t('page.ir.unblock') }}</VBtn>
        </VCardActions>
      </VCard>
    </VDialog>
  </div>
</template>

<style scoped>
.ir-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(120px, 1fr)); gap: 12px; }
.ir-device { padding: 16px 12px; text-align: center; cursor: pointer; }
.ir-device__no { font-size: 1.5rem; font-weight: 700; line-height: 1.2; }
.ir-device__label { font-size: 0.8125rem; margin-top: 4px; }
.ir-device--2 { animation: ir-blink 1.6s ease-in-out infinite; }
@keyframes ir-blink { 50% { opacity: 0.45; } }

/* 光電環：一格一顆，數字是光電編號；被遮蔽的閃紅（該台異常）或閃黃（該台正常，只是被東西擋著或亞健康） */
.ir-ring { display: inline-flex; flex-direction: column; gap: 4px; padding: 8px; border: 1px solid rgba(var(--v-border-color), var(--v-border-opacity)); border-radius: 8px; }
.ir-ring__row { display: flex; gap: 4px; }
.ir-cell { width: 44px; height: 36px; display: inline-flex; align-items: center; justify-content: center; border-radius: 6px; font-size: 0.8125rem; font-weight: 600; }
.ir-cell--empty { visibility: hidden; }
.ir-cell--off { background: rgba(var(--v-theme-success), 0.14); color: rgb(var(--v-theme-success)); }
.ir-cell--na { background: rgba(var(--v-theme-on-surface), 0.08); color: rgba(var(--v-theme-on-surface), 0.5); }
.ir-cell--on { background: rgb(var(--v-theme-warning)); color: #fff; animation: ir-blink 1.2s ease-in-out infinite; }
.ir-ring--abnormal .ir-cell--on { background: rgb(var(--v-theme-error)); }
.ir-cell--legend { width: 18px; height: 14px; vertical-align: middle; animation: none; }
</style>
