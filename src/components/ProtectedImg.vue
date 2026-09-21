<script setup>
/**
 * 要經過後台驗證的圖片（讀碼站照片）。
 *
 * 不能直接 `<img src="http://127.0.0.1:18090/…">`：桌面視窗的來源是 tauri://localhost，
 * WebKit 對 <img> 這種子資源不送 Origin／Referer，後台的跨站檢查會把它擋成 403。
 * 改用 fetch（會帶 Origin，桌面來源在放行名單內）拿回位元組再轉成 blob 網址給 <img>。
 */
import { apiBase } from '@/api/runtime'

const props = defineProps({ src: { type: String, required: true }, alt: { type: String, default: '' } })
const url = ref('')
const failed = ref(false)
let current = ''

const release = () => { if (current) { URL.revokeObjectURL(current); current = '' } }

watch(() => props.src, async src => {
  release()
  url.value = ''
  failed.value = false
  if (!src) return
  try {
    const res = await fetch(src.startsWith('http') ? src : apiBase() + src)
    if (!res.ok) throw new Error(String(res.status))
    current = URL.createObjectURL(await res.blob())
    url.value = current
  } catch {
    failed.value = true
  }
}, { immediate: true })

onBeforeUnmount(release)
</script>

<template>
  <img v-if="url" :src="url" :alt="alt" v-bind="$attrs">
  <div v-else class="protected-img__placeholder" v-bind="$attrs">
    <VIcon :icon="failed ? 'tabler-photo-off' : 'tabler-photo'" size="22" class="text-medium-emphasis" />
  </div>
</template>

<style scoped>
.protected-img__placeholder { display: inline-flex; align-items: center; justify-content: center; background: rgba(var(--v-theme-on-surface), 0.06); }
</style>
