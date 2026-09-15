<script setup>
/** 設定密碼對話框：配 useSettingsPassword() 使用 */
const props = defineProps({
  ctl: { type: Object, required: true },
})
const field = ref(null)
// VDialog 內的 autofocus 不可靠：開啟後主動聚焦，現場人員才能直接打密碼按 Enter
watch(() => props.ctl.dialog.value, open => { if (open) nextTick(() => setTimeout(() => field.value?.focus(), 150)) })
</script>

<template>
  <VDialog v-model="props.ctl.dialog.value" max-width="360" persistent>
    <VCard :title="$t('password.title')">
      <VCardText>
        <VTextField
          ref="field"
          v-model="props.ctl.input.value"
          :label="$t('password.label')"
          type="password"
          :error-messages="props.ctl.error.value"
          @keyup.enter="props.ctl.submit()"
        />
      </VCardText>
      <VCardActions>
        <VSpacer />
        <VBtn variant="text" @click="props.ctl.cancel()">{{ $t('common.cancel') }}</VBtn>
        <VBtn color="primary" @click="props.ctl.submit()">{{ $t('common.confirm') }}</VBtn>
      </VCardActions>
    </VCard>
  </VDialog>
</template>
