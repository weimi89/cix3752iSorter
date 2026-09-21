<script setup>
/**
 * 讀碼站照片放大檢視：獨立對話框，包裹查詢的照片按鈕與包裹詳情的縮圖都開它。
 * 只給 id 也行（列表只有 image_id），會自己去拿檔名與有沒有原圖。
 */
import { api, parcelImageUrl } from '@/api/http'
import ProtectedImg from '@/components/ProtectedImg.vue'

const props = defineProps({
  modelValue: Boolean,
  /** { id, file_name?, has_orig? }；只有 id 時會補查 */
  image: { type: Object, default: null },
})
const emit = defineEmits(['update:modelValue'])

const meta = ref(null)
const viewOriginal = ref(false)

watch(() => [props.modelValue, props.image], async ([open, img]) => {
  if (!open || !img) return
  viewOriginal.value = false
  meta.value = { id: img.id, file_name: '', has_orig: false }
  try { meta.value = await api.parcelImage(img.id) } catch { /* 拿不到就只顯示圖 */ }
}, { immediate: true })

const close = () => emit('update:modelValue', false)
</script>

<template>
  <VDialog :model-value="modelValue" max-width="1400" @update:model-value="v => { if (!v) close() }">
    <VCard v-if="meta">
      <VCardTitle class="d-flex align-center flex-wrap ga-2">
        <span v-if="meta.barcode" class="selectable text-title-large font-weight-bold">{{ meta.barcode }}</span>
        <VChip v-if="meta.chute_code" size="small" label variant="tonal" color="primary">{{ meta.chute_code }}</VChip>
        <span class="text-body-medium text-medium-emphasis">{{ meta.started_at || meta.received_at }}</span>
        <span class="text-body-small text-medium-emphasis selectable">{{ meta.file_name }}</span>
        <VSpacer />
        <VBtnToggle v-if="meta.has_orig" v-model="viewOriginal" density="compact" variant="outlined" mandatory class="me-3">
          <VBtn :value="false" size="small">{{ $t('parcel.imageEvidence') }}</VBtn>
          <VBtn :value="true" size="small">{{ $t('parcel.imageOriginal') }}</VBtn>
        </VBtnToggle>
        <VBtn icon variant="text" @click="close"><VIcon icon="tabler-x" /></VBtn>
      </VCardTitle>
      <VCardText class="pa-0"><ProtectedImg :src="parcelImageUrl(meta.id, viewOriginal)" :alt="meta.file_name" class="parcel-image__full" /></VCardText>
    </VCard>
  </VDialog>
</template>

<style scoped>
.parcel-image__full { display: block; inline-size: 100%; max-block-size: 85vh; object-fit: contain; background: #111; }
</style>
