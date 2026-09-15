<script setup>
/**
 * 頁首動作列：桌面直接放按鈕，手機收進「＋」選單（對齊 cix3752iLabelPrint 各頁的寫法）。
 * items: [{ key, label, icon, color?, variant?, loading?, disabled?, onClick }]
 */
defineProps({ items: { type: Array, required: true } })
</script>

<template>
  <div class="d-none d-md-flex ga-2">
    <VBtn v-for="a in items" :key="a.key" :color="a.color || 'primary'" :variant="a.variant || 'elevated'" :loading="a.loading" :disabled="a.disabled" @click="a.onClick">
      <VIcon :icon="a.icon" size="16" class="me-1" />{{ a.label }}
    </VBtn>
  </div>
  <VBtn class="d-block d-md-none" icon variant="tonal" color="default" density="compact" size="34">
    <VIcon icon="tabler-playlist-add" size="22" />
    <VMenu activator="parent">
      <VList>
        <VListItem v-for="a in items" :key="a.key" :disabled="a.disabled || a.loading" @click="a.onClick">
          <template #prepend><VIcon :icon="a.icon" size="20" /></template>
          <VListItemTitle>{{ a.label }}</VListItemTitle>
        </VListItem>
      </VList>
    </VMenu>
  </VBtn>
</template>
