<script setup>
/**
 * 系統設定：整份 config.toml。
 * - 每段一張卡（圖示標題＋副標），頂端有段落捷徑
 * - 有改動才能儲存；離開頁面前提醒；底部浮出未儲存提示
 * - 裝置段顯示目前連線狀態，可用「測試連線」試尚未儲存的位址
 * - 位址／網址格式即時檢查，錯誤時不送出
 */
import { useI18n } from 'vue-i18n'
import { onBeforeRouteLeave } from 'vue-router'
import { api } from '@/api/http'
import { useStatusStore } from '@/stores/status'
import AppHeader from '@/components/AppHeader.vue'
import PageActions from '@/components/PageActions.vue'
import { toast } from 'vue3-toastify'

const { t } = useI18n()
const status = useStatusStore()
const cfg = ref(null)
const saved = ref('')
const loading = ref(false)
const saving = ref(false)
const errorMsg = ref('')
const resetDialog = ref(false)
const testing = ref('')
const testResult = reactive({})

const SECTIONS = [
  { id: 'general', icon: 'tabler-adjustments', color: 'primary' },
  { id: 'middleware', icon: 'tabler-cloud-network', color: 'info' },
  { id: 'belt', icon: 'tabler-arrows-right', color: 'success' },
  { id: 'sorter', icon: 'tabler-route', color: 'success' },
  { id: 'camera', icon: 'tabler-scan', color: 'success' },
  { id: 'cameraFtp', icon: 'tabler-camera', color: 'success' },
  { id: 'led', icon: 'tabler-bulb', color: 'warning' },
  { id: 'rules', icon: 'tabler-hand-stop', color: 'error' },
  { id: 'print', icon: 'tabler-printer', color: 'secondary' },
  { id: 'buttons', icon: 'tabler-hand-click', color: 'secondary' },
  { id: 'webAccess', icon: 'tabler-world-www', color: 'info' },
]
const activeSection = ref('general')
// 捲到段落：目標要停在貼頂區塊下方，所以用貼頂區塊的實際底緣算位移
const jump = id => {
  activeSection.value = id
  if (id === SECTIONS[0].id) { window.scrollTo({ top: 0, behavior: 'smooth' }); return }
  const el = document.getElementById(`sec-${id}`)
  const stickyBottom = document.querySelector('.app-header-sticky')?.getBoundingClientRect().bottom ?? 0
  if (el) window.scrollTo({ top: window.scrollY + el.getBoundingClientRect().top - stickyBottom, behavior: 'smooth' })
}

const dirty = computed(() => !!cfg.value && JSON.stringify(cfg.value) !== saved.value)

const load = async () => {
  loading.value = true
  errorMsg.value = ''
  try {
    cfg.value = await api.config()
    lanCidrText.value = (cfg.value.web_access?.lan_cidrs || []).join('\n')
    saved.value = JSON.stringify(cfg.value)
    try { webPasswordSet.value = (await api.webAuthPassword()).password_set } catch { webPasswordSet.value = false }
  } catch (e) { errorMsg.value = e.message } finally { loading.value = false }
}

// ---- 網頁存取 ----
const webPasswordSet = ref(false)
const newWebPassword = ref('')
const showWebPassword = ref(false)
const savingWebPassword = ref(false)
/** 網段清單在畫面上以一行一筆呈現，比 JSON 陣列好讀好改 */
const lanCidrText = ref('')
const CIDR_RE = /^(\d{1,3}\.){3}\d{1,3}\/\d{1,2}$|^[0-9a-fA-F:]+\/\d{1,3}$/
const lanCidrList = computed(() => lanCidrText.value.split('\n').map(x => x.trim()).filter(Boolean))
watch(lanCidrList, list => { if (cfg.value?.web_access) cfg.value.web_access.lan_cidrs = list })
const cidrError = computed(() => (lanCidrList.value.every(c => CIDR_RE.test(c)) ? '' : t('page.settings.v.cidr')))

const saveWebPassword = async clear => {
  savingWebPassword.value = true
  try {
    const pw = clear ? '' : newWebPassword.value
    const r = await api.setWebAuthPassword(pw)
    webPasswordSet.value = !!r.password_set
    toast(pw ? t('page.settings.webAccess.passwordSaved') : t('page.settings.webAccess.passwordCleared'), { type: 'success' })
    newWebPassword.value = ''
  } catch (e) {
    toast(e.message, { type: 'error' })
  } finally { savingWebPassword.value = false }
}
const discard = () => { cfg.value = JSON.parse(saved.value); lanCidrText.value = (cfg.value.web_access?.lan_cidrs || []).join('\n') }

// ---- 驗證 ----
const ADDR_RE = /^(\d{1,3}\.){3}\d{1,3}:\d{1,5}$|^[a-zA-Z0-9.-]+:\d{1,5}$/
const addrError = v => (v && ADDR_RE.test(v.trim()) ? '' : t('page.settings.v.addr'))
const urlError = v => (/^https?:\/\/[^\s]+$/.test((v || '').trim()) ? '' : t('page.settings.v.url'))
const errors = computed(() => {
  if (!cfg.value) return []
  const list = []
  if (addrError(cfg.value.belt.addr)) list.push(`${t('device.belt')}：${t('page.settings.v.addr')}`)
  if (addrError(cfg.value.sorter.addr)) list.push(`${t('device.sorter')}：${t('page.settings.v.addr')}`)
  if (addrError(cfg.value.camera.listen)) list.push(`${t('device.camera')}：${t('page.settings.v.addr')}`)
  if (addrError(cfg.value.server.bind)) list.push(`${t('page.settings.general.bind')}：${t('page.settings.v.addr')}`)
  if (urlError(cfg.value.middleware.base_url)) list.push(`${t('page.settings.mw.baseUrl')}：${t('page.settings.v.url')}`)
  if (!cfg.value.general.default_chute?.trim()) list.push(t('page.settings.v.defaultChute'))
  if (cidrError.value) list.push(`${t('page.settings.webAccess.lanRanges')}：${t('page.settings.v.cidr')}`)
  for (const b of cfg.value.emergency_buttons) if (b.bit < 0 || b.bit > 7) list.push(`${t('page.settings.buttons.title')}「${b.describe}」：bit 0–7`)
  return list
})

const save = async () => {
  if (errors.value.length) { toast(errors.value[0], { type: 'error' }); return }
  saving.value = true
  try {
    await api.saveConfig(cfg.value)
    saved.value = JSON.stringify(cfg.value)
    toast(t('common.saved'), { type: 'success' })
  } catch (e) {
    toast(e.message, { type: 'error' })
  } finally { saving.value = false }
}

const testConn = async (target, addr) => {
  testing.value = target
  try {
    const r = await api.deviceTest(target, addr)
    testResult[target] = r
    toast(`${r.message}（${r.ms} ms）`, { type: r.ok ? 'success' : 'error' })
  } catch (e) { toast(e.message, { type: 'error' }) } finally { testing.value = '' }
}

const confirmReset = async () => {
  resetDialog.value = false
  try { await api.sorterReset(); toast(t('page.settings.resetSorterSent'), { type: 'success' }) } catch (e) { toast(e.message, { type: 'error' }) }
}

// ---- 列印 profile ----
const profileNames = computed(() => Object.keys(cfg.value?.print.profiles || {}))
const profileSize = name => { const m = name.match(/#(\d+)\*(\d+)/); return m ? `${m[1]} × ${m[2]} mm` : t('page.settings.print.noSize') }
const addProfile = () => { cfg.value.print.profiles[`PAPER-01#100*150-${profileNames.value.length + 1}`] = { cmd: 'DIRECTION 1', org: '0,0', retry: 30, retry_interval_ms: 200 } }
const renameProfile = (oldName, newName) => {
  newName = (newName || '').trim()
  if (!newName || newName === oldName || cfg.value.print.profiles[newName]) return
  const p = cfg.value.print.profiles[oldName]
  delete cfg.value.print.profiles[oldName]
  cfg.value.print.profiles[newName] = p
}
const removeProfile = name => { delete cfg.value.print.profiles[name] }
const addButton = () => cfg.value.emergency_buttons.push({ describe: '', device: 'belt', m2: 0, bit: 0, action: 'stop' })

const actions = computed(() => [
  { key: 'reset', label: t('page.settings.resetSorter'), icon: 'tabler-refresh-alert', color: 'warning', variant: 'outlined', onClick: () => { resetDialog.value = true } },
  { key: 'reload', label: t('common.reload'), icon: 'tabler-refresh', variant: 'outlined', loading: loading.value, onClick: load },
  { key: 'save', label: t('common.saveSettings'), icon: 'tabler-device-floppy', loading: saving.value, disabled: !dirty.value, onClick: save },
])

const deviceChip = key => status.devices[key]?.connected
onBeforeRouteLeave(() => { if (dirty.value && !confirm(t('page.settings.leaveConfirm'))) return false })
const beforeUnload = e => { if (dirty.value) { e.preventDefault(); e.returnValue = '' } }
onMounted(() => { load(); window.addEventListener('beforeunload', beforeUnload) })
onBeforeUnmount(() => window.removeEventListener('beforeunload', beforeUnload))
</script>

<template>
  <div class="settings-page">
    <!-- 頁首與段落捷徑一起貼頂：捲到哪都能存檔、換段 -->
    <AppHeader :title="$t('page.settings.title')" :subtitle="$t('page.settings.subtitle')" :subtitle-short="$t('page.settings.subtitleShort')" icon="tabler-settings" sticky>
      <template #actions><PageActions :items="actions" /></template>
      <template #below>
        <VCard v-if="cfg" class="section-nav">
          <VChipGroup v-model="activeSection" mandatory selected-class="text-primary" class="px-3 py-1">
            <VChip v-for="s in SECTIONS" :key="s.id" :value="s.id" size="small" variant="text" @click="jump(s.id)">
              <VIcon :icon="s.icon" size="16" start />{{ $t(`page.settings.sec.${s.id}`) }}
            </VChip>
          </VChipGroup>
        </VCard>
      </template>
    </AppHeader>
    <VAlert v-if="errorMsg" type="error" variant="tonal" class="mb-3">{{ errorMsg }}</VAlert>

    <template v-if="cfg">

      <VAlert v-if="errors.length" type="warning" variant="tonal" density="compact" class="mb-3" icon="tabler-alert-triangle">
        <div v-for="e in errors" :key="e">{{ e }}</div>
      </VAlert>

      <!-- 一般 -->
      <VCard id="sec-general" class="mb-4 card-shadow">
        <VCardTitle class="d-flex align-center justify-space-between px-4 py-3">
          <div class="d-flex align-center"><VIcon icon="tabler-adjustments" size="22" class="me-2" />{{ $t('page.settings.sec.general') }}</div>
        </VCardTitle>
        <VDivider />
        <VCardText class="pt-4">
          <VRow density="compact">
            <VCol cols="6" md="3"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.general.retention') }}</VLabel><VNumberInput v-model="cfg.general.retention_days" :min="0" :max="365" /></VCol>
            <VCol cols="6" md="3"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.general.defaultChute') }}</VLabel><VTextField v-model="cfg.general.default_chute" :error-messages="cfg.general.default_chute?.trim() ? '' : $t('page.settings.v.defaultChute')" /></VCol>
            <VCol cols="6" md="3"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.general.reentryHold') }}（ms）</VLabel><VNumberInput v-model="cfg.general.reentry_hold_ms" :min="0" :max="60000" :step="500" /></VCol>
          </VRow>
          <div class="text-body-small text-medium-emphasis mt-2">{{ $t('page.settings.general.reentryHoldHint') }}</div>
          <VDivider class="my-4" />
          <VRow density="compact" align="end">
            <VCol cols="12" md="4"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.general.bind') }}</VLabel><VTextField v-model="cfg.server.bind" :error-messages="addrError(cfg.server.bind)" /></VCol>
            <VCol cols="12" md="8"><div class="text-body-small text-medium-emphasis pb-2">{{ $t('page.settings.general.bindHint') }}</div></VCol>
          </VRow>
        </VCardText>
      </VCard>

      <!-- 中介機 -->
      <VCard id="sec-middleware" class="mb-4 card-shadow">
        <VCardTitle class="d-flex align-center justify-space-between px-4 py-3">
          <div class="d-flex align-center"><VIcon icon="tabler-cloud-network" size="22" class="me-2" />{{ $t('page.settings.mw.title') }}</div>
          <VBtn variant="tonal" size="small" :loading="testing === 'middleware'" @click="testConn('middleware', cfg.middleware.base_url)"><VIcon icon="tabler-plug" size="16" class="me-1" />{{ $t('page.settings.testConn') }}</VBtn>
        </VCardTitle>
        <VDivider />
        <VCardText class="pt-4">
          <VRow density="compact">
            <VCol cols="12" md="4"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.mw.baseUrl') }}</VLabel><VTextField v-model="cfg.middleware.base_url" :error-messages="urlError(cfg.middleware.base_url)" placeholder="http://192.168.0.37:18080/" /></VCol>
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.mw.parcelTimeout') }}（ms）</VLabel><VNumberInput v-model="cfg.middleware.parcel_timeout_ms" :min="200" :max="5000" :step="50" /></VCol>
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.mw.labelTimeout') }}（ms）</VLabel><VNumberInput v-model="cfg.middleware.label_timeout_ms" :min="200" :max="10000" :step="50" /></VCol>
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.mw.reportTimeout') }}（ms）</VLabel><VNumberInput v-model="cfg.middleware.report_timeout_ms" :min="500" :max="30000" :step="500" /></VCol>
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.mw.jamThrottle') }}（ms）</VLabel><VNumberInput v-model="cfg.middleware.jam_alert_throttle_ms" :min="1000" :max="600000" :step="1000" /></VCol>
          </VRow>
          <VAlert type="info" variant="tonal" density="compact" class="mt-3" icon="tabler-info-circle">{{ $t('page.settings.mw.parcelTimeoutHint') }}</VAlert>
        </VCardText>
      </VCard>

      <!-- 皮帶線 -->
      <VCard id="sec-belt" class="mb-4 card-shadow">
        <VCardTitle class="d-flex align-center justify-space-between px-4 py-3">
          <div class="d-flex align-center"><VIcon icon="tabler-arrows-right" size="22" class="me-2" />{{ $t('device.belt') }}</div>
          <div class="d-flex align-center ga-2">
            <VChip size="x-small" :color="deviceChip('belt') ? 'success' : 'error'" variant="tonal" label>{{ deviceChip('belt') ? $t('page.dashboard.connected') : $t('page.dashboard.disconnected') }}</VChip>
            <VBtn variant="tonal" size="small" :loading="testing === 'belt'" @click="testConn('belt', cfg.belt.addr)"><VIcon icon="tabler-plug" size="16" class="me-1" />{{ $t('page.settings.testConn') }}</VBtn>
          </div>
        </VCardTitle>
        <VDivider />
        <VCardText class="pt-4">
          <VRow density="compact">
            <VCol cols="12" md="4"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.addr') }}</VLabel><VTextField v-model="cfg.belt.addr" :error-messages="addrError(cfg.belt.addr)" placeholder="192.168.177.100:10006" /></VCol>
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.belt.run') }}</VLabel><VTextField v-model="cfg.belt.cmd.run" /></VCol>
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.belt.auto') }}</VLabel><VTextField v-model="cfg.belt.cmd.auto" /></VCol>
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.belt.stop') }}</VLabel><VTextField v-model="cfg.belt.cmd.stop" /></VCol>
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.belt.reset') }}</VLabel><VTextField v-model="cfg.belt.cmd.reset" /></VCol>
          </VRow>
          <VDivider class="my-4" />
          <div class="setting-row">
            <div><div class="text-body-large font-weight-medium">{{ $t('page.settings.belt.defaultRun') }}</div><div class="text-body-small text-medium-emphasis">{{ $t('page.settings.belt.defaultRunDesc') }}</div></div>
            <VSwitch v-model="cfg.belt.default_run" color="primary" hide-details />
          </div>
        </VCardText>
      </VCard>

      <!-- 分揀機 -->
      <VCard id="sec-sorter" class="mb-4 card-shadow">
        <VCardTitle class="d-flex align-center justify-space-between px-4 py-3">
          <div class="d-flex align-center"><VIcon icon="tabler-route" size="22" class="me-2" />{{ $t('device.sorter') }}</div>
          <div class="d-flex align-center ga-2">
            <VChip size="x-small" :color="deviceChip('sorter') ? 'success' : 'error'" variant="tonal" label>{{ deviceChip('sorter') ? $t('page.dashboard.connected') : $t('page.dashboard.disconnected') }}</VChip>
            <VBtn variant="tonal" size="small" :loading="testing === 'sorter'" @click="testConn('sorter', cfg.sorter.addr)"><VIcon icon="tabler-plug" size="16" class="me-1" />{{ $t('page.settings.testConn') }}</VBtn>
          </div>
        </VCardTitle>
        <VDivider />
        <VCardText class="pt-4">
          <VRow density="compact">
            <VCol cols="12" md="4"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.addr') }}</VLabel><VTextField v-model="cfg.sorter.addr" :error-messages="addrError(cfg.sorter.addr)" placeholder="192.168.177.198:10006" /></VCol>
            <VCol cols="4" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.sorter.speed') }}</VLabel><VNumberInput v-model="cfg.sorter.speed" :min="1" :max="300" /></VCol>
            <VCol cols="4" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.sorter.number') }}</VLabel><VNumberInput v-model="cfg.sorter.number" :min="1" :max="26" /></VCol>
            <VCol cols="4" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.sorter.irNum') }}</VLabel><VNumberInput v-model="cfg.sorter.ir_num" :min="0" :max="200" /></VCol>
          </VRow>
          <VExpansionPanels variant="accordion" class="mt-3 settings-advanced">
            <VExpansionPanel :title="$t('page.settings.advanced')">
              <VExpansionPanelText>
                <VRow density="compact">
                  <VCol cols="6" md="3"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.sorter.cTimeout') }}（ms）</VLabel><VNumberInput v-model="cfg.sorter.c_timeout_ms" :min="50" :max="5000" :step="50" /></VCol>
                  <VCol cols="6" md="3"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.sorter.kxAfterE') }}（ms）</VLabel><VNumberInput v-model="cfg.sorter.kx_after_e_ms" :min="0" :max="5000" :step="50" /></VCol>
                </VRow>
                <div class="setting-row mt-3">
                  <div><div class="text-body-large font-weight-medium">{{ $t('page.settings.sorter.uAsDone') }}</div><div class="text-body-small text-medium-emphasis">{{ $t('page.settings.sorter.uAsDoneDesc') }}</div></div>
                  <VSwitch v-model="cfg.sorter.u_as_done" color="primary" hide-details />
                </div>
                <div class="setting-row">
                  <div><div class="text-body-large font-weight-medium">{{ $t('page.settings.sorter.initOnI') }}</div><div class="text-body-small text-medium-emphasis">{{ $t('page.settings.sorter.initOnIDesc') }}</div></div>
                  <VSwitch v-model="cfg.sorter.init_parcel_on_i" color="primary" hide-details />
                </div>
              </VExpansionPanelText>
            </VExpansionPanel>
          </VExpansionPanels>
        </VCardText>
      </VCard>

      <!-- 讀碼站 -->
      <VCard id="sec-camera" class="mb-4 card-shadow">
        <VCardTitle class="d-flex align-center justify-space-between px-4 py-3">
          <div class="d-flex align-center"><VIcon icon="tabler-scan" size="22" class="me-2" />{{ $t('device.camera') }}</div>
          <div class="d-flex align-center ga-2">
            <VChip size="x-small" :color="deviceChip('camera') ? 'success' : 'error'" variant="tonal" label>{{ deviceChip('camera') ? $t('page.dashboard.connected') : $t('page.dashboard.disconnected') }}</VChip>
          </div>
        </VCardTitle>
        <VDivider />
        <VCardText class="pt-4">
          <VRow density="compact">
            <VCol cols="12" md="4"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.camera.listen') }}</VLabel><VTextField v-model="cfg.camera.listen" :error-messages="addrError(cfg.camera.listen)" placeholder="0.0.0.0:8051" /></VCol>
            <VCol cols="4" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.camera.floor') }}（ms）</VLabel><VNumberInput v-model="cfg.camera.bind_floor_ms" :min="-5000" :max="0" :step="50" /></VCol>
            <VCol cols="4" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.camera.ceiling') }}（ms）</VLabel><VNumberInput v-model="cfg.camera.bind_ceiling_ms" :min="100" :max="10000" :step="100" /></VCol>
            <VCol cols="4" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.camera.dedup') }}（ms）</VLabel><VNumberInput v-model="cfg.camera.dedup_ms" :min="0" :max="60000" :step="500" /></VCol>
          </VRow>
          <div class="text-body-small text-medium-emphasis mt-2">{{ $t('page.settings.camera.hint') }}</div>
          <VRow density="compact" class="mt-1">
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.camera.frameWidth') }}（px）</VLabel><VNumberInput v-model="cfg.camera.frame_width" :min="0" :max="20000" :step="100" /></VCol>
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.camera.frameHeight') }}（px）</VLabel><VNumberInput v-model="cfg.camera.frame_height" :min="0" :max="20000" :step="100" /></VCol>
          </VRow>
          <div class="text-body-small text-medium-emphasis mt-2">{{ $t('page.settings.camera.frameHint') }}</div>
        </VCardText>
      </VCard>

      <!-- 讀碼站照片（FTP 收圖） -->
      <VCard id="sec-cameraFtp" class="mb-4 card-shadow">
        <VCardTitle class="d-flex align-center justify-space-between px-4 py-3">
          <div class="d-flex align-center"><VIcon icon="tabler-camera" size="22" class="me-2" />{{ $t('page.settings.sec.cameraFtp') }}</div>
        </VCardTitle>
        <VDivider />
        <VCardText class="pt-4">
          <div class="text-body-small text-medium-emphasis mb-3">{{ $t('page.settings.cameraFtp.desc') }}</div>
          <div class="setting-row mb-3">
            <div>
              <div class="text-body-large font-weight-medium">{{ $t('page.settings.cameraFtp.enable') }}</div>
              <div class="text-body-small text-medium-emphasis">{{ $t('page.settings.cameraFtp.enableDesc') }}</div>
            </div>
            <VSwitch v-model="cfg.camera_ftp.enabled" class="flex-shrink-0" hide-details color="primary" inset />
          </div>
          <VRow density="compact">
            <VCol cols="12" md="4"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.cameraFtp.listen') }}</VLabel><VTextField v-model="cfg.camera_ftp.listen" :error-messages="addrError(cfg.camera_ftp.listen)" placeholder="0.0.0.0:2121" /></VCol>
            <VCol cols="6" md="4"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.cameraFtp.username') }}</VLabel><VTextField v-model="cfg.camera_ftp.username" autocomplete="off" /></VCol>
            <VCol cols="6" md="4"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.cameraFtp.password') }}</VLabel><VTextField v-model="cfg.camera_ftp.password" autocomplete="off" /></VCol>
            <VCol cols="6" md="3"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.cameraFtp.passiveMin') }}</VLabel><VNumberInput v-model="cfg.camera_ftp.passive_port_min" :min="0" :max="65535" :step="1" /></VCol>
            <VCol cols="6" md="3"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.cameraFtp.passiveMax') }}</VLabel><VNumberInput v-model="cfg.camera_ftp.passive_port_max" :min="0" :max="65535" :step="1" /></VCol>
            <VCol cols="6" md="3"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.cameraFtp.maxEdge') }}（px）</VLabel><VNumberInput v-model="cfg.camera_ftp.max_edge_px" :min="0" :max="8000" :step="100" /></VCol>
            <VCol cols="6" md="3"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.cameraFtp.quality') }}</VLabel><VNumberInput v-model="cfg.camera_ftp.jpeg_quality" :min="1" :max="100" :step="5" /></VCol>
            <VCol cols="6" md="3"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.cameraFtp.window') }}（ms）</VLabel><VNumberInput v-model="cfg.camera_ftp.match_window_ms" :min="500" :max="60000" :step="500" /></VCol>
            <VCol cols="12" md="6"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.cameraFtp.imagesDir') }}</VLabel><VTextField v-model="cfg.camera_ftp.images_dir" :placeholder="$t('page.settings.cameraFtp.imagesDirPlaceholder')" /></VCol>
            <VCol cols="6" md="3"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.cameraFtp.retention') }}</VLabel><VNumberInput v-model="cfg.camera_ftp.retention_days" :min="0" :max="3650" :step="1" /></VCol>
          </VRow>
          <div class="setting-row mt-3">
            <div>
              <div class="text-body-large font-weight-medium">{{ $t('page.settings.cameraFtp.keepOriginalNoread') }}</div>
              <div class="text-body-small text-medium-emphasis">{{ $t('page.settings.cameraFtp.keepOriginalNoreadDesc') }}</div>
            </div>
            <VSwitch v-model="cfg.camera_ftp.keep_original_noread" class="flex-shrink-0" hide-details color="primary" inset />
          </div>
          <div class="text-body-small text-medium-emphasis mt-3">{{ $t('page.settings.cameraFtp.hint') }}</div>
        </VCardText>
      </VCard>

      <!-- 系統燈 -->
      <VCard id="sec-led" class="mb-4 card-shadow">
        <VCardTitle class="d-flex align-center justify-space-between px-4 py-3">
          <div class="d-flex align-center"><VIcon icon="tabler-bulb" size="22" class="me-2" />{{ $t('page.settings.led.title') }}</div>
        </VCardTitle>
        <VDivider />
        <VCardText class="pt-4">
          <VRow density="compact">
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.led.via') }}</VLabel><VSelect v-model="cfg.sysled.via" :items="[{ value: 'sorter', title: $t('device.sorter') }, { value: 'belt', title: $t('device.belt') }]" /></VCol>
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.led.green') }}</VLabel><VTextField v-model="cfg.sysled.cmd.green" /></VCol>
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.led.red') }}</VLabel><VTextField v-model="cfg.sysled.cmd.red" /></VCol>
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.led.idle') }}</VLabel><VTextField v-model="cfg.sysled.cmd.idle" /></VCol>
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.led.off') }}</VLabel><VTextField v-model="cfg.sysled.cmd.off" /></VCol>
          </VRow>
          <VRow density="compact" class="mt-1">
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.led.hold') }}（ms）</VLabel><VNumberInput v-model="cfg.sysled.alarm_hold_ms" :min="500" :max="60000" :step="500" /></VCol>
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.led.idleAfter') }}（ms）</VLabel><VNumberInput v-model="cfg.sysled.idle_after_ms" :min="1000" :max="600000" :step="1000" /></VCol>
          </VRow>
          <VDivider class="my-4" />
          <VRow density="compact">
            <VCol v-for="k in ['alarm_on_start', 'alarm_on_block', 'alarm_on_lost', 'alarm_on_cancel']" :key="k" cols="6" md="3">
              <VSwitch v-model="cfg.sysled[k]" :label="$t(`page.settings.led.${k}`)" color="primary" hide-details />
            </VCol>
          </VRow>
        </VCardText>
      </VCard>

      <!-- 停線規則 -->
      <VCard id="sec-rules" class="mb-4 card-shadow">
        <VCardTitle class="d-flex align-center justify-space-between px-4 py-3">
          <div class="d-flex align-center"><VIcon icon="tabler-hand-stop" size="22" class="me-2" />{{ $t('page.settings.rules.title') }}</div>
        </VCardTitle>
        <VDivider />
        <VList density="compact" class="py-0">
          <template v-for="(k, i) in ['on_block', 'on_lost', 'on_cancel', 'on_i', 'too_close']" :key="k">
            <VDivider v-if="i" />
            <VListItem>
              <VListItemTitle class="text-body-large font-weight-medium">{{ $t(`page.settings.rules.${k}`) }}</VListItemTitle>
              <VListItemSubtitle class="text-body-small">{{ $t(`page.settings.rules.${k}Desc`) }}</VListItemSubtitle>
              <template #append>
                <div class="d-flex align-center ga-3">
                  <template v-if="k === 'too_close'">
                    <span class="text-body-small text-medium-emphasis">{{ $t('page.settings.rules.minGap') }}</span>
                    <VNumberInput v-model="cfg.ng.min_gap" :min="0" :max="999" :disabled="!cfg.ng.too_close" density="compact" hide-details style="inline-size: 130px" />
                  </template>
                  <VSwitch v-model="cfg.ng[k]" color="error" hide-details />
                </div>
              </template>
            </VListItem>
          </template>
        </VList>
      </VCard>

      <!-- 列印 -->
      <VCard id="sec-print" class="mb-4 card-shadow">
        <VCardTitle class="d-flex align-center justify-space-between px-4 py-3">
          <div class="d-flex align-center"><VIcon icon="tabler-printer" size="22" class="me-2" />{{ $t('page.settings.print.title') }}</div>
          <VBtn variant="tonal" size="small" @click="addProfile"><VIcon icon="tabler-plus" size="16" class="me-1" />{{ $t('page.settings.print.addProfile') }}</VBtn>
        </VCardTitle>
        <VDivider />
        <VCardText class="pt-4">
          <VRow density="compact" class="mb-2">
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">DPI</VLabel><VNumberInput v-model="cfg.print.dpi" :min="72" :max="600" /></VCol>
            <VCol cols="6" md="3"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.print.delay') }}（ms）</VLabel><VNumberInput v-model="cfg.print.per_chute_delay_ms" :min="0" :max="10000" :step="100" /></VCol>
            <VCol cols="12" md="7" class="d-flex align-end"><div class="text-body-small text-medium-emphasis pb-2">{{ $t('page.settings.print.delayHint') }}</div></VCol>
          </VRow>
          <VRow density="compact">
            <VCol v-for="name in profileNames" :key="name" cols="12" md="6" xl="4">
              <VCard variant="outlined" class="h-100">
                <VCardTitle class="d-flex align-center justify-space-between px-4 py-2 text-body-large">
                  <div class="d-flex align-center"><VIcon icon="tabler-file-text" size="20" class="me-2" />{{ name }}<span class="text-body-small text-medium-emphasis ms-2">{{ profileSize(name) }}</span></div>
                  <VBtn icon variant="text" size="small" color="error" @click="removeProfile(name)"><VIcon icon="tabler-trash" size="20" /><VTooltip activator="parent" location="bottom">{{ $t('common.delete') }}</VTooltip></VBtn>
                </VCardTitle>
        <VDivider />
                <VCardText class="pt-4">
                  <VRow density="compact">
                    <VCol cols="12"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.print.profileName') }}</VLabel><VTextField :model-value="name" density="compact" @change="renameProfile(name, $event.target.value)" /></VCol>
                    <VCol cols="12"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.print.cmd') }}</VLabel><VTextField v-model="cfg.print.profiles[name].cmd" density="compact" /></VCol>
                    <VCol cols="4"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.print.org') }}</VLabel><VTextField v-model="cfg.print.profiles[name].org" density="compact" /></VCol>
                    <VCol cols="4"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.print.retry') }}</VLabel><VNumberInput v-model="cfg.print.profiles[name].retry" :min="1" :max="999" density="compact" /></VCol>
                    <VCol cols="4"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.print.retryInterval') }}</VLabel><VNumberInput v-model="cfg.print.profiles[name].retry_interval_ms" :min="50" :max="60000" :step="50" density="compact" /></VCol>
                  </VRow>
                </VCardText>
              </VCard>
            </VCol>
          </VRow>
          <div class="text-body-small text-medium-emphasis mt-3">{{ $t('page.settings.print.profileHint') }}</div>
        </VCardText>
      </VCard>

      <!-- 急停按鈕 -->
      <VCard id="sec-buttons" class="mb-4 card-shadow">
        <VCardTitle class="d-flex align-center justify-space-between px-4 py-3">
          <div class="d-flex align-center"><VIcon icon="tabler-hand-click" size="22" class="me-2" />{{ $t('page.settings.buttons.title') }}</div>
          <VBtn variant="tonal" size="small" @click="addButton"><VIcon icon="tabler-plus" size="16" class="me-1" />{{ $t('common.add') }}</VBtn>
        </VCardTitle>
        <VDivider />
        <VCardText class="pt-4">
          <div v-if="!cfg.emergency_buttons.length" class="py-4 d-flex align-center justify-center text-medium-emphasis"><VIcon icon="tabler-alert-circle" size="20" class="me-1" />{{ $t('page.settings.buttons.empty') }}</div>
          <VRow v-for="(b, i) in cfg.emergency_buttons" :key="i" density="compact" align="end" class="mb-1">
            <VCol cols="12" md="4"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.buttons.describe') }}</VLabel><VTextField v-model="b.describe" density="compact" /></VCol>
            <VCol cols="6" md="2"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.buttons.device') }}</VLabel><VSelect v-model="b.device" :items="[{ value: 'belt', title: $t('device.belt') }, { value: 'sorter', title: $t('device.sorter') }]" density="compact" /></VCol>
            <VCol cols="3" md="1"><VLabel class="mb-1 text-body-medium">m2</VLabel><VNumberInput v-model="b.m2" :min="0" :max="99" density="compact" control-variant="hidden" /></VCol>
            <VCol cols="3" md="1"><VLabel class="mb-1 text-body-medium">bit</VLabel><VNumberInput v-model="b.bit" :min="0" :max="7" density="compact" control-variant="hidden" /></VCol>
            <VCol cols="9" md="3"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.buttons.action') }}</VLabel><VSelect v-model="b.action" :items="[{ value: 'stop', title: $t('page.settings.buttons.stop') }, { value: 'start', title: $t('page.settings.buttons.start') }, { value: 'estop', title: $t('page.settings.buttons.estop') }, { value: 'estop_release', title: $t('page.settings.buttons.estopRelease') }]" density="compact" /></VCol>
            <VCol cols="3" md="1" class="text-end"><VBtn icon variant="text" size="small" color="error" @click="cfg.emergency_buttons.splice(i, 1)"><VIcon icon="tabler-trash" size="20" /></VBtn></VCol>
          </VRow>
        </VCardText>
      </VCard>
      <!-- 網頁存取 -->
      <VCard v-if="cfg.web_access" id="sec-webAccess" class="mb-4 card-shadow">
        <VCardTitle class="d-flex align-center justify-space-between px-4 py-3">
          <div class="d-flex align-center"><VIcon icon="tabler-world-www" size="22" class="me-2" />{{ $t('page.settings.sec.webAccess') }}</div>
        </VCardTitle>
        <VDivider />
        <VCardText class="pt-4">
          <div class="text-body-small text-medium-emphasis mb-3">{{ $t('page.settings.webAccess.desc') }}</div>
          <div class="setting-row mb-3">
            <div>
              <div class="text-body-large font-weight-medium">{{ $t('page.settings.webAccess.enable') }}</div>
              <div class="text-body-small text-medium-emphasis">{{ $t('page.settings.webAccess.enableDesc') }}</div>
            </div>
            <VSwitch v-model="cfg.web_access.enabled" class="flex-shrink-0" hide-details color="primary" inset />
          </div>
          <VAlert type="warning" variant="tonal" density="compact" class="mb-3" icon="tabler-shield-exclamation">{{ $t('page.settings.webAccess.noEncryptionWarning') }}</VAlert>
          <VAlert type="info" variant="tonal" density="compact" class="mb-4" icon="tabler-info-circle">{{ $t('page.settings.webAccess.lanOnlyNote') }}</VAlert>

          <VDivider class="mb-4" />
          <div class="text-body-large font-weight-medium mb-1">{{ $t('page.settings.webAccess.passwordTitle') }}</div>
          <div class="text-body-small text-medium-emphasis mb-3">{{ webPasswordSet ? $t('page.settings.webAccess.passwordIsSet') : $t('page.settings.webAccess.passwordNotSet') }}</div>
          <VRow density="compact" align="end">
            <VCol cols="12" md="5">
              <VLabel class="mb-1 text-body-medium">{{ $t('page.settings.webAccess.newPassword') }}</VLabel>
              <VTextField v-model="newWebPassword" :type="showWebPassword ? 'text' : 'password'" :append-inner-icon="showWebPassword ? 'tabler-eye-off' : 'tabler-eye'" autocomplete="new-password" density="compact" @click:append-inner="showWebPassword = !showWebPassword" />
            </VCol>
            <VCol cols="12" md="7" class="d-flex ga-2 flex-wrap">
              <VBtn color="primary" :loading="savingWebPassword" :disabled="newWebPassword.trim().length < 8" @click="saveWebPassword(false)"><VIcon icon="tabler-key" size="16" class="me-1" />{{ $t('page.settings.webAccess.savePassword') }}</VBtn>
              <VBtn v-if="webPasswordSet" variant="tonal" color="error" :loading="savingWebPassword" @click="saveWebPassword(true)">{{ $t('page.settings.webAccess.clearPassword') }}</VBtn>
            </VCol>
          </VRow>
          <div class="text-body-small text-medium-emphasis mt-1 mb-4">{{ $t('page.settings.webAccess.passwordHint') }}</div>

          <VDivider class="mb-4" />
          <VRow density="compact">
            <VCol cols="12" sm="4"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.webAccess.sessionHours') }}</VLabel><VNumberInput v-model="cfg.web_access.session_hours" :min="1" :max="720" density="compact" :hint="$t('page.settings.webAccess.sessionHoursHint')" persistent-hint /></VCol>
            <VCol cols="12" sm="4"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.webAccess.maxFails') }}</VLabel><VNumberInput v-model="cfg.web_access.max_fail_attempts" :min="1" :max="50" density="compact" :hint="$t('page.settings.webAccess.maxFailsHint')" persistent-hint /></VCol>
            <VCol cols="12" sm="4"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.webAccess.lockMinutes') }}</VLabel><VNumberInput v-model="cfg.web_access.lock_minutes" :min="1" :max="1440" density="compact" :hint="$t('page.settings.webAccess.lockMinutesHint')" persistent-hint /></VCol>
            <VCol cols="12"><VLabel class="mb-1 text-body-medium">{{ $t('page.settings.webAccess.lanRanges') }}</VLabel><VTextarea v-model="lanCidrText" rows="4" auto-grow density="compact" :error-messages="cidrError" :hint="$t('page.settings.webAccess.lanRangesHint')" persistent-hint /></VCol>
          </VRow>
        </VCardText>
      </VCard>
    </template>

    <!-- 未儲存提示：固定在底部，捲到哪都看得到 -->
    <VSlideYReverseTransition>
      <div v-if="dirty" class="unsaved-bar">
        <VIcon icon="tabler-alert-circle" color="warning" class="me-2" />
        <span class="text-body-medium">{{ $t('page.settings.unsaved') }}</span>
        <VSpacer />
        <VBtn variant="text" @click="discard">{{ $t('page.settings.discard') }}</VBtn>
        <VBtn color="primary" :loading="saving" @click="save"><VIcon icon="tabler-device-floppy" size="16" class="me-1" />{{ $t('common.saveSettings') }}</VBtn>
      </div>
    </VSlideYReverseTransition>

    <VDialog v-model="resetDialog" max-width="420">
      <VCard>
        <VCardTitle>{{ $t('page.settings.resetSorter') }}</VCardTitle>
        <VCardText>{{ $t('page.settings.resetSorterConfirm') }}</VCardText>
        <VCardActions>
          <VSpacer />
          <VBtn variant="text" @click="resetDialog = false">{{ $t('common.cancel') }}</VBtn>
          <VBtn color="warning" variant="flat" @click="confirmReset">{{ $t('common.confirm') }}</VBtn>
        </VCardActions>
      </VCard>
    </VDialog>
  </div>
</template>

<style scoped lang="scss">
.settings-page { padding-block-end: 72px; }


.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}

.unsaved-bar {
  position: fixed;
  inset-inline: 0;
  inset-block-end: 0;
  z-index: 10;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 24px;
  padding-inline-start: calc(var(--v-layout-left, 0px) + 24px);
  background: rgb(var(--v-theme-surface));
  border-top: 1px solid rgb(var(--v-theme-on-surface) / 0.12);
  box-shadow: 0 -4px 12px rgb(0 0 0 / 0.06);
}

.settings-advanced :deep(.v-expansion-panel-title) { font-size: 0.9rem; min-block-size: 40px; }
</style>
