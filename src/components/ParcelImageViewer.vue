<script setup>
/**
 * 讀碼站照片放大檢視：Viewer.js 全螢幕檢視器（滾輪／手勢縮放、拖曳、旋轉、翻轉、1:1），
 * 包裹查詢、包裹詳情、異常存證的縮圖都開它。只給 id 也行（列表只有 image_id），會自己去拿檔名。
 *
 * 圖片先 fetch 成 blob 再交給檢視器：桌面視窗的來源是 tauri://localhost，<img> 直接載後台網址會被跨站防護擋成 403
 * （同 ProtectedImg）。有原圖的舊件直接看原圖；現在全部存原圖，不再讓人切證據圖／原圖。
 */
import Viewer from 'viewerjs'
import 'viewerjs/dist/viewer.css'
import { api, parcelImageUrl } from '@/api/http'
import { apiBase } from '@/api/runtime'

const props = defineProps({
  modelValue: Boolean,
  /** { id, file_name?, has_orig? }；只有 id 時會補查 */
  image: { type: Object, default: null },
})
const emit = defineEmits(['update:modelValue'])

const holder = ref(null)
let viewer = null
let blobUrl = ''
let opening = 0

const destroy = () => {
  if (viewer) { viewer.destroy(); viewer = null }
  if (blobUrl) { URL.revokeObjectURL(blobUrl); blobUrl = '' }
  if (holder.value) holder.value.innerHTML = ''
}

const open = async img => {
  const seq = ++opening
  destroy()
  let meta = { id: img.id, file_name: '', has_orig: false }
  try { meta = await api.parcelImage(img.id) } catch { /* 拿不到就只顯示圖 */ }
  if (seq !== opening) return
  const src = parcelImageUrl(meta.id, !!meta.has_orig)
  try {
    const res = await fetch(src.startsWith('http') ? src : apiBase() + src)
    if (!res.ok) throw new Error(String(res.status))
    blobUrl = URL.createObjectURL(await res.blob())
  } catch {
    emit('update:modelValue', false)
    return
  }
  if (seq !== opening || !holder.value) return
  const el = document.createElement('img')
  el.src = blobUrl
  el.alt = meta.file_name || ''
  holder.value.appendChild(el)
  const title = [meta.barcode, meta.chute_code, meta.started_at || meta.received_at, meta.file_name].filter(Boolean).join(' · ')
  viewer = new Viewer(el, {
    navbar: false,
    title: () => title,
    // 單張：不要上一張／下一張與播放
    toolbar: { zoomIn: 1, zoomOut: 1, oneToOne: 1, reset: 1, rotateLeft: 1, rotateRight: 1, flipHorizontal: 1, flipVertical: 1 },
    zIndex: 3000,
    transition: false,
    hidden: () => emit('update:modelValue', false),
  })
  viewer.show()
}

watch(() => [props.modelValue, props.image], ([isOpen, img]) => {
  if (isOpen && img) open(img)
  else { opening++; destroy() }
}, { immediate: true })

onBeforeUnmount(() => { opening++; destroy() })
</script>

<template>
  <!-- 檢視器自己會開全螢幕遮罩；這個容器只用來掛那張隱藏的 img -->
  <div ref="holder" class="parcel-image-viewer__holder" />
</template>

<style scoped>
.parcel-image-viewer__holder { display: none; }
</style>
