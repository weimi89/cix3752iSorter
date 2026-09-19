<script setup>
/** 格口對照：左右兩欄卡片（對齊 cix3752iLabelPrint「分揀通道」頁），其餘特殊口另成一區。 */
import { useI18n } from 'vue-i18n'
import { api } from '@/api/http'
import AppHeader from '@/components/AppHeader.vue'
import PageActions from '@/components/PageActions.vue'
import { toast } from 'vue3-toastify'

const { t } = useI18n()
const list = ref([])
const printers = ref([])
const loading = ref(false)
const saving = ref(false)
const errorMsg = ref('')
const dirty = ref(false)

const load = async () => {
  loading.value = true
  errorMsg.value = ''
  try {
    list.value = await api.chutes()
    const p = await api.printers()
    printers.value = p.attached.map(a => a.port)
    dirty.value = false
  } catch (e) { errorMsg.value = e.message } finally { loading.value = false }
}

const portItems = computed(() => {
  const set = new Set(printers.value)
  for (const c of list.value) if (c.printer_port) set.add(c.printer_port)
  return [...set].sort()
})
const printerMissing = c => !!c.printer_port && !printers.value.includes(c.printer_port)

const side = c => /^L\d/.test(c.code) ? 'left' : /^R\d/.test(c.code) ? 'right' : 'other'
const numOf = c => Number((c.code.match(/\d+/) || [0])[0])
const sorted = s => list.value.filter(c => side(c) === s).sort((a, b) => numOf(a) - numOf(b) || a.sort_order - b.sort_order)
const left = computed(() => sorted('left'))
const right = computed(() => sorted('right'))
const other = computed(() => sorted('other'))

const add = () => { list.value.push({ code: '', label: '', cid: 1000323, printer_port: null, enabled: true, sort_order: (list.value.at(-1)?.sort_order || 0) + 1 }); dirty.value = true }
const remove = c => { list.value.splice(list.value.indexOf(c), 1); dirty.value = true }
const mark = () => { dirty.value = true }

const save = async () => {
  saving.value = true
  try {
    await api.saveChutes(list.value.map(c => ({ ...c, cid: Number(c.cid), sort_order: Number(c.sort_order), bag_limit: Number(c.bag_limit) || 0 })))
    toast(t('common.saved'), { type: 'success' })
    load()
  } catch (e) {
    toast(e.message, { type: 'error' })
  } finally { saving.value = false }
}

const actions = computed(() => [
  { key: 'add', label: t('common.add'), icon: 'tabler-plus', variant: 'outlined', onClick: add },
  { key: 'reload', label: t('common.reload'), icon: 'tabler-refresh', variant: 'outlined', loading: loading.value, onClick: load },
  { key: 'save', label: t('common.save'), icon: 'tabler-device-floppy', loading: saving.value, disabled: !dirty.value, onClick: save },
])

onMounted(load)
</script>

<template>
  <div>
    <AppHeader :title="$t('page.chutes.title')" :subtitle="$t('page.chutes.subtitle')" :subtitle-short="$t('page.chutes.subtitleShort')" icon="tabler-route" sticky>
      <template #actions><PageActions :items="actions" /></template>
    </AppHeader>
    <VAlert v-if="errorMsg" type="error" variant="tonal" class="mb-3">{{ errorMsg }}</VAlert>
    <VAlert type="info" variant="tonal" class="mb-3" icon="tabler-info-circle">{{ $t('page.chutes.hint') }}</VAlert>

    <VRow>
      <VCol v-for="col in [{ key: 'left', items: left, icon: 'tabler-arrow-narrow-left', cls: 'channel-card--left' }, { key: 'right', items: right, icon: 'tabler-arrow-narrow-right', cls: 'channel-card--right' }]" :key="col.key" cols="12" md="6">
        <div class="column-label">
          <VIcon :icon="col.icon" size="18" :color="col.key === 'left' ? 'primary' : 'warning'" />
          <span>{{ $t(`page.chutes.${col.key}Column`) }}</span>
        </div>
        <div class="d-flex flex-column ga-4">
          <div v-for="c in col.items" :key="c.code || c.sort_order" class="channel-card" :class="[col.cls, { 'channel-card--paused': !c.enabled }]">
            <div class="channel-card__head">
              <VIcon class="channel-card__icon" :icon="col.icon" size="18" :color="col.key === 'left' ? 'primary' : 'warning'" />
              <span class="channel-card__title">{{ c.code || '—' }}</span>
              <VChip v-if="!c.enabled" size="x-small" color="warning" variant="flat" class="channel-card__chip">{{ $t('page.chutes.disabled') }}</VChip>
              <VChip v-else-if="printerMissing(c)" size="x-small" color="error" variant="tonal" class="channel-card__chip">{{ $t('page.chutes.printerOffline') }}</VChip>
              <VChip v-else size="x-small" color="success" variant="tonal" class="channel-card__chip">{{ $t('page.chutes.enabled') }}</VChip>
              <VBtn icon variant="text" size="small" color="error" class="channel-card__delete" @click="remove(c)">
                <VIcon icon="tabler-trash" size="20" />
                <VTooltip activator="parent" location="bottom">{{ $t('common.delete') }}</VTooltip>
              </VBtn>
            </div>
            <VRow no-gutters class="mx-n1">
              <VCol cols="6" md="3" class="px-1"><div class="search-field"><label>{{ $t('page.chutes.code') }}</label><VTextField v-model="c.code" density="compact" variant="outlined" hide-details @update:model-value="mark" /></div></VCol>
              <VCol cols="6" md="3" class="px-1"><div class="search-field"><label>{{ $t('page.chutes.label') }}</label><VTextField v-model="c.label" density="compact" variant="outlined" hide-details @update:model-value="mark" /></div></VCol>
              <VCol cols="6" md="3" class="px-1"><div class="search-field"><label>CID</label><VNumberInput v-model="c.cid" :min="1000000" :max="1999999" :step="1" control-variant="hidden" density="compact" variant="outlined" hide-details @update:model-value="mark" /></div></VCol>
              <VCol cols="6" md="3" class="px-1"><div class="search-field"><label>{{ $t('page.chutes.order') }}</label><VNumberInput v-model="c.sort_order" :min="0" density="compact" variant="outlined" hide-details @update:model-value="mark" /></div></VCol>
              <VCol cols="6" md="3" class="px-1"><div class="search-field"><label>{{ $t('page.chutes.bagLimit') }}</label><VNumberInput v-model="c.bag_limit" :min="0" :step="10" density="compact" variant="outlined" hide-details @update:model-value="mark" /></div></VCol>
              <VCol cols="12" class="px-1"><div class="search-field"><label>{{ $t('page.chutes.printer') }}</label><VCombobox v-model="c.printer_port" :items="portItems" :placeholder="$t('page.chutes.noPrinter')" :error="printerMissing(c)" density="compact" variant="outlined" hide-details clearable @update:model-value="mark" />
                <div v-if="printerMissing(c)" class="text-body-small text-error mt-1 d-flex align-center"><VIcon icon="tabler-alert-triangle" size="13" class="me-1" />{{ $t('page.chutes.printerMissingHint') }}</div></div></VCol>
            </VRow>
            <div class="pause-row" :class="c.enabled ? 'pause-row--on' : 'pause-row--off'">
              <VIcon :icon="c.enabled ? 'tabler-player-play' : 'tabler-player-pause'" size="20" :color="c.enabled ? 'success' : 'warning'" />
              <span class="pause-row__text">{{ c.enabled ? $t('page.chutes.enabled') : $t('page.chutes.disabled') }}</span>
              <VSwitch v-model="c.enabled" class="pause-row__switch" color="success" hide-details density="compact" @update:model-value="mark" />
            </div>
          </div>
        </div>
      </VCol>
    </VRow>

    <div class="column-label mt-4">
      <VIcon icon="tabler-arrows-split-2" size="18" color="secondary" />
      <span>{{ $t('page.chutes.otherColumn') }}</span>
    </div>
    <VRow>
      <VCol v-for="c in other" :key="c.code || c.sort_order" cols="12" md="4">
        <div class="channel-card" :class="{ 'channel-card--paused': !c.enabled }">
          <div class="channel-card__head">
            <VIcon class="channel-card__icon" icon="tabler-arrows-split-2" size="18" color="secondary" />
            <span class="channel-card__title">{{ c.code || '—' }}</span>
            <VChip v-if="!c.enabled" size="x-small" color="warning" variant="flat" class="channel-card__chip">{{ $t('page.chutes.disabled') }}</VChip>
            <VChip v-else size="x-small" color="success" variant="tonal" class="channel-card__chip">{{ $t('page.chutes.enabled') }}</VChip>
            <VBtn icon variant="text" size="small" color="error" class="channel-card__delete" @click="remove(c)">
              <VIcon icon="tabler-trash" size="20" />
              <VTooltip activator="parent" location="bottom">{{ $t('common.delete') }}</VTooltip>
            </VBtn>
          </div>
          <VRow no-gutters class="mx-n1">
            <VCol cols="6" class="px-1"><div class="search-field"><label>{{ $t('page.chutes.code') }}</label><VTextField v-model="c.code" density="compact" variant="outlined" hide-details @update:model-value="mark" /></div></VCol>
            <VCol cols="6" class="px-1"><div class="search-field"><label>{{ $t('page.chutes.label') }}</label><VTextField v-model="c.label" density="compact" variant="outlined" hide-details @update:model-value="mark" /></div></VCol>
            <VCol cols="6" class="px-1"><div class="search-field"><label>CID</label><VNumberInput v-model="c.cid" :min="1000000" :max="1999999" :step="1" control-variant="hidden" density="compact" variant="outlined" hide-details @update:model-value="mark" /></div></VCol>
            <VCol cols="6" class="px-1"><div class="search-field"><label>{{ $t('page.chutes.order') }}</label><VNumberInput v-model="c.sort_order" :min="0" density="compact" variant="outlined" hide-details @update:model-value="mark" /></div></VCol>
            <VCol cols="6" class="px-1"><div class="search-field"><label>{{ $t('page.chutes.bagLimit') }}</label><VNumberInput v-model="c.bag_limit" :min="0" :step="10" density="compact" variant="outlined" hide-details @update:model-value="mark" /></div></VCol>
            <VCol cols="12" class="px-1"><div class="search-field"><label>{{ $t('page.chutes.printer') }}</label><VCombobox v-model="c.printer_port" :items="portItems" :placeholder="$t('page.chutes.noPrinter')" density="compact" variant="outlined" hide-details clearable @update:model-value="mark" /></div></VCol>
          </VRow>
          <div class="pause-row" :class="c.enabled ? 'pause-row--on' : 'pause-row--off'">
            <VIcon :icon="c.enabled ? 'tabler-player-play' : 'tabler-player-pause'" size="20" :color="c.enabled ? 'success' : 'warning'" />
            <span class="pause-row__text">{{ c.enabled ? $t('page.chutes.enabled') : $t('page.chutes.disabled') }}</span>
            <VSwitch v-model="c.enabled" class="pause-row__switch" color="success" hide-details density="compact" @update:model-value="mark" />
          </div>
        </div>
      </VCol>
    </VRow>
  </div>
</template>

<style scoped lang="scss">
.column-label {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 12px;
  font-weight: 600;
  font-size: 0.95rem;
}

.channel-card {
  position: relative;
  border: 1px solid rgb(var(--v-theme-on-surface) / 0.08);
  border-radius: 8px;
  padding: 10px 12px 12px;
  background: rgb(var(--v-theme-surface));
  box-shadow: 0 2px 6px rgb(0 0 0 / 0.06), 0 1px 2px rgb(0 0 0 / 0.04);
  transition: border-color 0.15s, box-shadow 0.15s, background 0.15s;

  &:hover { box-shadow: 0 4px 12px rgb(0 0 0 / 0.08), 0 2px 4px rgb(0 0 0 / 0.05); }
  &--left { border-left: 3px solid rgb(var(--v-theme-primary)); }
  &--right { border-right: 3px solid rgb(var(--v-theme-warning)); }
  &--paused {
    background: repeating-linear-gradient(135deg, rgb(var(--v-theme-warning) / 0.05), rgb(var(--v-theme-warning) / 0.05) 8px, rgb(var(--v-theme-warning) / 0.1) 8px, rgb(var(--v-theme-warning) / 0.1) 16px);
    .channel-card__title { opacity: 0.6; }
  }
  &__head { display: flex; align-items: center; gap: 8px; margin-bottom: 8px; }
  &__icon { flex-shrink: 0; }
  &__title { font-weight: 600; font-size: 0.95rem; line-height: 1; }
  &__chip { margin-left: auto; font-size: 0.7rem; }
  &__delete { margin-inline-start: 4px; margin-block: -6px; }
  :deep(.search-field) {
    margin-bottom: 8px;
    label { font-size: 12px; }
  }
}

.pause-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 4px;
  padding: 4px 6px 4px 10px;
  border-radius: 6px;
  border: 1px solid rgb(var(--v-theme-on-surface) / 0.08);
  &__text { font-size: 0.875rem; font-weight: 600; }
  &__switch { margin-left: auto; flex: 0 0 auto; }
  &--on .pause-row__text { color: rgb(var(--v-theme-success)); }
  &--off .pause-row__text { color: rgb(var(--v-theme-warning)); }
}
</style>
