<script setup>
import { useTheme } from 'vuetify'
import { hexToRgb } from '@layouts/utils'
import { useRoute } from 'vue-router'
import DefaultLayout from '@/layouts/DefaultLayout.vue'
import { useThemeApply } from '@/composables/useThemeApply'

// 主題（主色、半暗側欄）與 cix3752iLabelPrint 同一套設定
useThemeApply()
const { global } = useTheme()
const route = useRoute()
</script>

<template>
  <VApp :style="`--v-global-theme-primary: ${hexToRgb(global.current.value.colors.primary)}`">
    <!-- 登入頁不套版面：還沒通過這道門的人不該看到側欄，也不該啟動全域輪詢 -->
    <RouterView v-if="route.name === 'login'" />
    <DefaultLayout v-else>
      <RouterView />
    </DefaultLayout>
  </VApp>
</template>
