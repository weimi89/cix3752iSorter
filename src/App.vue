<script setup>
import { useTheme } from 'vuetify'
import { hexToRgb } from '@layouts/utils'
import DefaultLayout from '@/layouts/DefaultLayout.vue'
import { useThemeApply } from '@/composables/useThemeApply'

// 主題（主色、半暗側欄）與 cix3752iLabelPrint 同一套設定
useThemeApply()
const { global } = useTheme()
</script>

<template>
  <VApp :style="`--v-global-theme-primary: ${hexToRgb(global.current.value.colors.primary)}`">
    <DefaultLayout>
      <RouterView />
    </DefaultLayout>
  </VApp>
</template>
