<script setup>
/**
 * Table 分頁元件 — 結構完全對齊 Materio resources/js/components/TablePagination.vue
 */

const props = defineProps({
  page: { type: Number, required: true },
  perPage: { type: Number, required: true },
  total: { type: Number, required: true },
  pageSizes: { type: Array, default: () => [10, 25, 50, 100, 250, 500] },
  header: { type: Boolean, default: false },
})

const emit = defineEmits(['update:page', 'update:perPage'])

const totalPages = computed(() => Math.max(1, Math.ceil(props.total / props.perPage)))
const pageOptions = computed(() => Array.from({ length: totalPages.value }, (_, i) => i + 1))

const setPage = n => {
  const clamped = Math.max(1, Math.min(totalPages.value, n))
  if (clamped !== props.page) emit('update:page', clamped)
}

const onFirst = () => setPage(1)
const onPrev = () => setPage(props.page - 1)
const onNext = () => setPage(props.page + 1)
const onLast = () => setPage(totalPages.value)

const separator = n => new Intl.NumberFormat('en-US').format(n || 0)
</script>

<template>
  <!-- 版型對齊 cix3752iWeb 的 TablePagination:
       桌面兩塊都顯示(左:總筆數 + 每頁;右:第 N 頁 + 翻頁);
       手機(<768)上下各留一半 —— 頁首只放翻頁、頁尾只放筆數與每頁,避免同一組控制項在手機出現兩次 -->
  <div class="pager-root py-2 px-6">
    <div class="d-flex flex-wrap ga-2 align-center justify-center">
      <div class="pager-jump d-flex align-center ga-2 ml-md-auto" :class="{ 'pager-hide-mobile': !header }">
        <div>{{ $t('pagination.pagePrefix') }}</div>
        <div class="pager-select">
          <VSelect
            density="compact"
            :items="pageOptions"
            :model-value="page"
            @update:model-value="setPage"
          />
        </div>
        <div>{{ $t('pagination.pageSuffix') }}</div>
        <div class="pager-nav d-flex ga-1">
          <VBtn class="pager-nav__edge" icon variant="text" size="small" :disabled="page === 1" @click="onFirst">
            <VIcon icon="tabler-player-skip-back" size="22" />
          </VBtn>
          <VBtn icon variant="text" size="small" :disabled="page === 1" @click="onPrev">
            <VIcon icon="tabler-chevron-left" size="22" />
          </VBtn>
          <VBtn icon variant="text" size="small" :disabled="page === totalPages" @click="onNext">
            <VIcon icon="tabler-chevron-right" size="22" />
          </VBtn>
          <VBtn class="pager-nav__edge" icon variant="text" size="small" :disabled="page === totalPages" @click="onLast">
            <VIcon icon="tabler-player-skip-forward" size="22" />
          </VBtn>
        </div>
      </div>

      <div class="d-flex flex-wrap ga-2 align-center justify-center order-sm-first" :class="{ 'pager-hide-mobile': header }">
        <span class="pager-summary d-md-none d-lg-flex text-nowrap">
          {{ $t('pagination.summary', { total: separator(total), pages: separator(totalPages) }) }}
        </span>
        <div class="d-flex align-center ga-2">
          <span class="text-nowrap">{{ $t('pagination.perPagePrefix') }}</span>
          <div class="pager-select">
            <VSelect
              density="compact"
              :items="pageSizes.map(String)"
              :model-value="String(perPage)"
              @update:model-value="v => emit('update:perPage', Number(v))"
            />
          </div>
          <span class="text-nowrap">{{ $t('pagination.perPageSuffix') }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped lang="scss">
.text-nowrap {
  white-space: nowrap !important;
}

// 下拉選單本身沒有固有寬度,擺在被壓縮的列裡會縮到只剩箭頭、看不見頁碼;給固定的最小寬度並禁止被壓縮
.pager-select {
  flex: 0 0 auto;
  min-inline-size: 4.75rem;
}

// 手機:頁首、頁尾各只留一塊(見 template 註解)
@media (max-width: 767.98px) {
  .pager-hide-mobile {
    display: none !important;
  }

  // 「總計 N 筆記錄分為 M 頁」越南文比卡片還寬,讓它折行
  .pager-summary {
    white-space: normal !important;
    text-align: center;
  }
}

// 很窄的螢幕連四顆翻頁鍵都放不下,收掉「第一頁 / 最後一頁」
@media (max-width: 399.98px) {
  .pager-nav__edge {
    display: none;
  }
}
</style>
