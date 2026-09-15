<script setup>
/** 發現新版本對話框（版型對齊 cix3752iLabelPrint 導覽列的更新對話框），多了「上傳更新檔」給沒外網的工控機 */
import { useUpdater } from '@/composables/useUpdater'

const model = defineModel({ type: Boolean, default: false })
const { updateInfo, isDownloading, downloadProgress, stage, lastError, downloadAndInstall, uploadAndInstall, dismissUpdate, canUpload } = useUpdater()
const fileInput = ref(null)
const pickFile = () => fileInput.value?.click()
const onFile = e => { const f = e.target.files?.[0]; if (f) uploadAndInstall(f); e.target.value = '' }
const stageText = computed(() => ({ downloading: 'updater.downloading', installing: 'updater.installing', restarting: 'updater.restarting' }[stage.value] || 'updater.downloading'))
</script>

<template>
  <VDialog v-model="model" max-width="480" persistent>
    <VCard>
      <VCardTitle class="d-flex align-center px-4 py-3 bg-grey-300">
        <VIcon icon="tabler-arrow-up-circle" color="warning" class="me-2" />
        {{ $t('updater.title') }}
      </VCardTitle>
      <VCardText class="pa-4">
        <div class="mb-2">
          <span class="text-body-medium text-medium-emphasis">{{ $t('updater.current') }}</span>
          <strong class="ms-1">v{{ updateInfo?.currentVersion }}</strong>
          <VIcon icon="tabler-arrow-right" size="16" class="mx-2" />
          <span class="text-body-medium text-medium-emphasis">{{ $t('updater.next') }}</span>
          <strong class="ms-1 text-warning">v{{ updateInfo?.version }}</strong>
        </div>
        <div v-if="updateInfo?.notes" class="text-body-medium mt-3 pa-3 rounded bg-grey-100"><pre class="ma-0" style="white-space: pre-wrap;">{{ updateInfo.notes }}</pre></div>
        <VProgressLinear v-if="isDownloading" :model-value="stage === 'downloading' ? downloadProgress : 100" :indeterminate="stage !== 'downloading'" color="warning" height="8" rounded class="mt-4" />
        <div v-if="isDownloading" class="text-body-small text-medium-emphasis mt-1">{{ $t(stageText) }}<template v-if="stage === 'downloading'"> {{ downloadProgress }}%</template></div>
        <VAlert v-if="lastError" type="error" variant="tonal" density="compact" class="mt-3">{{ lastError }}</VAlert>
        <template v-if="canUpload">
          <VDivider class="my-4" />
          <div class="text-body-small text-medium-emphasis">{{ $t('updater.uploadHint') }}</div>
          <input ref="fileInput" type="file" accept=".gz,.tgz" hidden @change="onFile">
        </template>
      </VCardText>
      <VCardActions class="px-4 pb-4 flex-wrap ga-2">
        <VBtn v-if="canUpload" variant="text" :disabled="isDownloading" @click="pickFile"><VIcon icon="tabler-upload" size="16" class="me-1" />{{ $t('updater.upload') }}</VBtn>
        <VSpacer />
        <VBtn variant="text" :disabled="isDownloading" @click="() => { model = false; dismissUpdate() }">{{ $t('updater.later') }}</VBtn>
        <VBtn color="warning" variant="flat" :loading="isDownloading" :disabled="isDownloading" @click="downloadAndInstall"><VIcon icon="tabler-download" size="16" class="me-1" />{{ $t('updater.install') }}</VBtn>
      </VCardActions>
    </VCard>
  </VDialog>
</template>
