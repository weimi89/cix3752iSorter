<script setup>
/**
 * 頁首卡：圖示／標題／說明／動作列。
 * `sticky` 讓整張頁首（含 `below` 插槽，例如段落捷徑列）貼在頂欄下方，
 * 設定類頁面捲到底也能按到儲存。貼頂位置從頂欄實際位置量出來，頂欄高度變（手機、捲動壓縮）也跟著對。
 */
const props = defineProps({
  title: { type: String, required: true },
  subtitle: { type: String, default: '' },
  // 手機用的短版說明:標題旁只有一行多的寬度,長說明會疊成三四行把整張頁首撐高。
  // 沒給就沿用 subtitle
  subtitleShort: { type: String, default: '' },
  icon: { type: String, default: '' },
  sticky: { type: Boolean, default: false },
})

const stickyTop = ref('0px')
let ro = null
const measure = () => {
  const nav = document.querySelector('.layout-navbar')
  if (!nav) return
  // 頂欄本身是 sticky：它貼住時的位置＝computed top，加上高度就是頁首該貼的位置；
  // 與頂欄之間的 12px 縫由容器的 padding-top 補，才不會透出捲過去的內容
  const top = parseFloat(getComputedStyle(nav).insetBlockStart || getComputedStyle(nav).top) || 0
  stickyTop.value = `${Math.round(top + nav.getBoundingClientRect().height)}px`
}
onMounted(() => {
  if (!props.sticky) return
  measure()
  ro = new ResizeObserver(measure)
  const nav = document.querySelector('.layout-navbar')
  if (nav) ro.observe(nav)
  window.addEventListener('scroll', measure, { passive: true })
})
onBeforeUnmount(() => {
  ro?.disconnect()
  window.removeEventListener('scroll', measure)
})
</script>

<template>
  <div :class="{ 'app-header-sticky': sticky }" :style="sticky ? { top: stickyTop } : undefined">
    <div class="app-header-card">
      <!-- 圖示與文字自成一組（間距 12px），與右側動作列之間才是 24px -->
      <div class="app-header-card__lead">
        <VIcon
          v-if="icon"
          :icon="icon"
          class="app-header-card__icon"
        />
        <div class="app-header-card__text">
          <span class="app-header-card__title">{{ title }}</span>
          <span v-if="subtitle" class="app-header-card__subtitle d-none d-sm-inline">{{ subtitle }}</span>
          <span v-if="subtitleShort || subtitle" class="app-header-card__subtitle d-sm-none">{{ subtitleShort || subtitle }}</span>
        </div>
      </div>
      <VSpacer />
      <div class="app-header-card__actions">
        <slot name="actions" />
      </div>
    </div>
    <slot name="below" />
  </div>
</template>
