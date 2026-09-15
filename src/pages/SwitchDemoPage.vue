<script setup>
const on = ref(true)
const variants = [
  { key: 'A', label: 'A：原生（inset=false）', props: {} },
  { key: 'B', label: 'B：原生 compact', props: { density: 'compact' } },
  { key: 'C', label: 'C：inset（Materio 預設）', props: { inset: true } },
  { key: 'D', label: 'D：inset compact', props: { inset: true, density: 'compact' } },
  { key: 'E', label: 'E：inset 帶勾叉圖示', props: { inset: true, trueIcon: 'tabler-check', falseIcon: 'tabler-x' } },
  { key: 'F', label: 'F：inset compact 帶圖示', props: { inset: true, density: 'compact', trueIcon: 'tabler-check', falseIcon: 'tabler-x' } },
  { key: 'G', label: 'G：原生 flat（無陰影）', props: { flat: true } },
  { key: 'H', label: 'H：原生 compact 帶圖示', props: { density: 'compact', trueIcon: 'tabler-check', falseIcon: 'tabler-x' } },
]
</script>
<template>
  <VCard class="pa-4">
    <div class="text-body-large font-weight-medium mb-2">VSwitch 樣式比較（每一列左：開、右：關）</div>
    <VRow>
      <VCol v-for="v in variants" :key="v.key" cols="12" md="6" lg="3">
        <div class="pa-3 rounded" style="border: 1px solid rgb(var(--v-theme-on-surface) / 0.12)">
          <div class="text-body-medium mb-2">{{ v.label }}</div>
          <div class="d-flex align-center ga-6">
            <VSwitch :model-value="true" color="success" v-bind="v.props" hide-details />
            <VSwitch :model-value="false" color="success" v-bind="v.props" hide-details />
          </div>
        </div>
      </VCol>
    </VRow>
  </VCard>
</template>
