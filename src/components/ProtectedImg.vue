<script setup>
/**
 * 要經過後台驗證的圖片（讀碼站照片）。
 *
 * 不能直接 `<img src="http://127.0.0.1:18090/…">`：桌面視窗的來源是 tauri://localhost，
 * WebKit 對 <img> 這種子資源不送 Origin／Referer，後台的跨站檢查會把它擋成 403。
 * 改用 fetch（會帶 Origin，桌面來源在放行名單內）拿回位元組再轉成 blob 網址給 <img>。
 */
import { apiBase } from '@/api/runtime'
import { useLatestRequest } from '@/composables/useLatestRequest'

const props = defineProps({ src: { type: String, required: true }, alt: { type: String, default: '' } })
// 載入失敗時通知使用端（例如面單預覽要改顯示說明）
const emit = defineEmits(['error'])
const url = ref('')
const failed = ref(false)
let current = ''
// 下載途中換了圖（翻頁、切篩選）或元件已拆掉：晚到的那張不建 blob 網址——
// 建了就沒人釋放，照片是原圖、一張幾百 KB，翻幾頁就漏掉幾十 MB；也不能蓋掉新的那張
const latest = useLatestRequest()
let disposed = false

const release = () => { if (current) { URL.revokeObjectURL(current); current = '' } }

watch(() => props.src, async src => {
  const id = latest.start()
  release()
  url.value = ''
  failed.value = false
  if (!src) return
  try {
    const res = await fetch(src.startsWith('http') ? src : apiBase() + src)
    if (!res.ok) throw new Error(String(res.status))
    const blob = await res.blob()
    if (disposed || !latest.isLatest(id)) return
    current = URL.createObjectURL(blob)
    url.value = current
  } catch {
    if (!disposed && latest.isLatest(id)) { failed.value = true; emit('error') }
  }
}, { immediate: true })

onBeforeUnmount(() => { disposed = true; release() })
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
