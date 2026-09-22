<script setup>
import { useI18n } from 'vue-i18n'
import { api } from '@/api/http'
import AppHeader from '@/components/AppHeader.vue'
import AppDatePicker from '@/components/AppDatePicker.vue'
import PageActions from '@/components/PageActions.vue'
import { fmtDate } from '@/composables/useFormat'

const { t } = useI18n()

// 一天一頁：主管不會盯著看板，事後看這頁一眼就要知道要改什麼。日期空白＝最近有件的那天
const day = ref('')
const data = ref(null)
const loading = ref(false)
const errorMsg = ref('')
const today = () => fmtDate(new Date())

const load = async () => {
  loading.value = true
  errorMsg.value = ''
  try {
    data.value = await api.reportDay(day.value)
    if (day.value !== data.value.day) day.value = data.value.day
  } catch (e) { errorMsg.value = e.message } finally { loading.value = false }
}
watch(day, (v, old) => { if (v && v !== old && v !== data.value?.day) load() })
onMounted(load)

const o = computed(() => data.value?.overview)
const range = computed(() => o.value?.range || { total: 0, done: 0, abnormal: 0, middleware: 0, noread_landed: 0 })
const doneNormal = computed(() => range.value.done - range.value.middleware - range.value.noread_landed)
const doneRate = computed(() => (range.value.total ? Math.round((doneNormal.value / range.value.total) * 1000) / 10 : null))
const kpis = computed(() => [
  { key: 'total', value: range.value.total, color: 'primary', icon: 'tabler-packages' },
  { key: 'doneRate', value: doneRate.value === null ? '—' : doneRate.value + '%', color: doneRate.value === null ? 'secondary' : doneRate.value >= 97 ? 'success' : doneRate.value >= 94 ? 'warning' : 'error', icon: 'tabler-circle-check' },
  { key: 'sorter', value: range.value.abnormal, color: 'error', icon: 'tabler-alert-circle' },
  { key: 'middleware', value: range.value.middleware, color: 'info', icon: 'tabler-cloud-x' },
  { key: 'noread', value: range.value.noread_landed, color: 'warning', icon: 'tabler-barcode-off' },
])

const findings = computed(() => data.value?.findings || [])
const LEVEL = { error: { color: 'error', icon: 'tabler-alert-octagon' }, warning: { color: 'warning', icon: 'tabler-alert-triangle' }, info: { color: 'secondary', icon: 'tabler-info-circle' } }
// 每條問題點連到能看細節的地方
const gotoFor = f => {
  const d = data.value?.day
  switch (f.code) {
    case 'noread': case 'reentry': return { name: 'abnormal', query: { day: d, kind: 'noread' } }
    case 'sorter': return { name: 'abnormal', query: { day: d, kind: 'sorter' } }
    case 'middleware': return { name: 'abnormal', query: { day: d, kind: 'middleware' } }
    default: return { name: 'stats', query: { day: d } }
  }
}

const noreadCauses = computed(() => o.value?.noread_causes || [])
const noreadCausesTotal = computed(() => noreadCauses.value.reduce((a, b) => a + b.count, 0))
const refedChutes = computed(() => (o.value?.by_chute || []).filter(c => c.refed_after > 0).sort((a, b) => b.refed_after - a.refed_after))
const refedMax = computed(() => refedChutes.value.reduce((a, b) => Math.max(a, b.refed_after), 0))
const jamModules = computed(() => (o.value?.jams?.by_module || []).slice().sort((a, b) => b.count - a.count))
const jamMax = computed(() => jamModules.value.reduce((a, b) => Math.max(a, b.count), 0))
const pct = (n, max) => (max > 0 ? Math.round((n / max) * 100) : 0)
const share = (n, total) => (total ? Math.round((n / total) * 100) : 0)

const actions = computed(() => [
  { key: 'reload', label: t('common.reload'), icon: 'tabler-refresh', loading: loading.value, onClick: load },
])
</script>

<template>
  <div>
    <AppHeader :title="$t('page.report.title')" :subtitle="$t('page.report.subtitle')" :subtitle-short="$t('page.report.subtitleShort')" icon="tabler-report-analytics">
      <template #actions><PageActions :items="actions" /></template>
    </AppHeader>

    <VAlert v-if="errorMsg" type="error" variant="tonal" class="mb-3">{{ errorMsg }}</VAlert>

    <!-- 日期：前一天／後一天只跳有件的日子 -->
    <VCard class="mb-3 card-shadow">
      <VCardText class="d-flex flex-wrap align-center gap-3">
        <VBtn variant="tonal" size="small" :disabled="!data?.prev_day" @click="day = data.prev_day"><VIcon icon="tabler-chevron-left" start />{{ $t('page.report.prevDay') }}</VBtn>
        <div style="inline-size: 180px;"><AppDatePicker v-model="day" :label="$t('page.report.day')" :max="today()" /></div>
        <VBtn variant="tonal" size="small" :disabled="!data?.next_day" @click="day = data.next_day">{{ $t('page.report.nextDay') }}<VIcon icon="tabler-chevron-right" end /></VBtn>
      </VCardText>
    </VCard>

    <template v-if="data && !range.total">
      <VCard class="card-shadow"><VCardText class="py-8 text-center text-medium-emphasis">{{ $t('page.report.noData') }}</VCardText></VCard>
    </template>
    <template v-else-if="data">
      <!-- 大字數字 -->
      <VRow density="compact">
        <VCol v-for="k in kpis" :key="k.key" cols="6" md="auto" class="flex-md-grow-1">
          <VCard class="card-shadow h-100">
            <VCardText class="d-flex align-center gap-3">
              <VAvatar :color="k.color" variant="tonal" size="44"><VIcon :icon="k.icon" size="24" /></VAvatar>
              <div>
                <div class="text-body-small text-medium-emphasis">{{ $t(`page.report.kpi.${k.key}`) }}</div>
                <div class="report-kpi font-weight-bold" :class="`text-${k.color}`">{{ k.value }}</div>
              </div>
            </VCardText>
          </VCard>
        </VCol>
      </VRow>

      <!-- 問題點 -->
      <VCard class="mt-3 card-shadow">
        <VCardItem>
          <template #prepend><VAvatar :color="findings.length ? (findings[0].level === 'error' ? 'error' : 'warning') : 'success'" variant="tonal"><VIcon icon="tabler-list-check" /></VAvatar></template>
          <VCardTitle>{{ $t('page.report.findings') }}<VChip v-if="findings.length" size="x-small" color="error" class="ms-2" label>{{ findings.length }}</VChip></VCardTitle>
          <VCardSubtitle>{{ $t('page.report.findingsHint') }}</VCardSubtitle>
        </VCardItem>
        <VDivider />
        <VCardText v-if="!findings.length" class="py-8 text-center"><VIcon icon="tabler-mood-smile" size="40" class="text-success mb-2" /><div class="text-medium-emphasis">{{ $t('page.report.noFindings') }}</div></VCardText>
        <VCardText v-else class="pt-3">
          <div v-for="(f, i) in findings" :key="f.code" class="finding" :class="`finding--${f.level}`">
            <div class="finding__head">
              <VIcon :icon="LEVEL[f.level].icon" :color="LEVEL[f.level].color" size="22" />
              <div class="finding__title">{{ i + 1 }}. {{ f.title }}</div>
              <RouterLink :to="gotoFor(f)" class="finding__link text-no-wrap">{{ $t(`page.report.goto.${f.code}`) }}<VIcon icon="tabler-arrow-right" size="14" class="ms-1" /></RouterLink>
            </div>
            <div class="finding__detail">{{ f.detail }}</div>
            <div class="finding__action"><VIcon icon="tabler-bulb" size="16" class="me-1" />{{ $t('page.report.action') }}：{{ f.action }}</div>
          </div>
        </VCardText>
      </VCard>

      <!-- 三張小卡：讀碼失敗原因、格口又進線、卡件模組 -->
      <VRow density="compact" class="mt-1">
        <VCol cols="12" md="4">
          <VCard class="card-shadow h-100">
            <VCardItem><VCardTitle class="text-body-large font-weight-bold">{{ $t('page.report.cards.causes') }}</VCardTitle></VCardItem>
            <VDivider />
            <VCardText>
              <div v-if="!noreadCauses.length" class="text-body-small text-medium-emphasis">{{ $t('page.report.cards.causesEmpty') }}</div>
              <div v-else class="mini-rows">
                <div v-for="r in noreadCauses" :key="r.key" class="mini-row">
                  <div class="mini-row__label">{{ $t(`page.abnormal.cause.${r.key}`) }}</div>
                  <div class="mini-row__bar"><div class="mini-row__fill bg-warning" :style="{ inlineSize: pct(r.count, noreadCauses[0].count) + '%' }" /></div>
                  <div class="mini-row__value">{{ r.count }} <span class="text-medium-emphasis">({{ share(r.count, noreadCausesTotal) }}%)</span></div>
                </div>
              </div>
            </VCardText>
          </VCard>
        </VCol>
        <VCol cols="12" md="4">
          <VCard class="card-shadow h-100">
            <VCardItem><VCardTitle class="text-body-large font-weight-bold">{{ $t('page.report.cards.refed') }}</VCardTitle></VCardItem>
            <VDivider />
            <VCardText>
              <div v-if="!refedChutes.length" class="text-body-small text-medium-emphasis">{{ $t('page.report.cards.refedEmpty') }}</div>
              <div v-else class="mini-rows">
                <div v-for="c in refedChutes" :key="c.code" class="mini-row">
                  <div class="mini-row__label">{{ c.code }} <span class="text-medium-emphasis">{{ c.label }}</span></div>
                  <div class="mini-row__bar"><div class="mini-row__fill bg-error" :style="{ inlineSize: pct(c.refed_after, refedMax) + '%' }" /></div>
                  <div class="mini-row__value">{{ c.refed_after }} <span class="text-medium-emphasis">({{ c.done ? Math.round((c.refed_after / c.done) * 1000) / 10 : 0 }}%)</span></div>
                </div>
              </div>
            </VCardText>
          </VCard>
        </VCol>
        <VCol cols="12" md="4">
          <VCard class="card-shadow h-100">
            <VCardItem><VCardTitle class="text-body-large font-weight-bold">{{ $t('page.report.cards.jams') }}</VCardTitle></VCardItem>
            <VDivider />
            <VCardText>
              <div v-if="!jamModules.length" class="text-body-small text-medium-emphasis">{{ $t('page.report.cards.jamsEmpty') }}</div>
              <div v-else class="mini-rows">
                <div v-for="m in jamModules" :key="m.key" class="mini-row">
                  <div class="mini-row__label">{{ m.key }}</div>
                  <div class="mini-row__bar"><div class="mini-row__fill bg-warning" :style="{ inlineSize: pct(m.count, jamMax) + '%' }" /></div>
                  <div class="mini-row__value">{{ m.count }} <span class="text-medium-emphasis">({{ share(m.count, o.jams.total) }}%)</span></div>
                </div>
              </div>
            </VCardText>
          </VCard>
        </VCol>
      </VRow>
    </template>
  </div>
</template>

<style scoped lang="scss">
.report-kpi { font-size: 2rem; line-height: 2.25rem; white-space: nowrap; }
// 問題點：左邊色條分級，標題大字，動作一行
.finding {
  padding: 12px 14px;
  border-inline-start: 4px solid rgb(var(--v-theme-secondary));
  border-radius: 6px;
  background: rgba(var(--v-theme-on-surface), 0.03);
  & + & { margin-block-start: 10px; }
  &--error { border-color: rgb(var(--v-theme-error)); background: rgba(var(--v-theme-error), 0.06); }
  &--warning { border-color: rgb(var(--v-theme-warning)); background: rgba(var(--v-theme-warning), 0.06); }
  &__head { display: flex; align-items: center; gap: 8px; }
  &__title { font-size: 1.05rem; font-weight: 700; flex: 1; min-inline-size: 0; }
  &__link { font-size: 0.8rem; }
  &__detail { margin-block-start: 4px; padding-inline-start: 30px; color: rgba(var(--v-theme-on-surface), 0.75); font-size: 0.9rem; }
  &__action { margin-block-start: 4px; padding-inline-start: 30px; font-size: 0.9rem; font-weight: 600; }
}
.mini-rows { display: flex; flex-direction: column; gap: 6px; }
.mini-row { display: grid; grid-template-columns: 120px 1fr 90px; align-items: center; gap: 8px; font-size: 12px; }
.mini-row__label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
.mini-row__bar { block-size: 8px; background: rgba(var(--v-theme-on-surface), 0.05); border-radius: 4px; overflow: hidden; }
.mini-row__fill { block-size: 100%; }
.mini-row__value { text-align: end; font-variant-numeric: tabular-nums; white-space: nowrap; font-weight: 600; }
@media (max-width: 599px) {
  .report-kpi { font-size: 1.5rem; line-height: 1.75rem; }
  .finding__head { flex-wrap: wrap; }
  .finding__detail, .finding__action { padding-inline-start: 0; }
}
</style>
