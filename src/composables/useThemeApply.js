import { watch } from 'vue'
import { useTheme } from 'vuetify'
import { useThemeStore, darken } from '@/stores/theme'
import { useLayoutConfigStore } from '@layouts/stores/config'
import { useConfigStore } from '@core/stores/config'

/**
 * 把 pinia themeStore 的設定同步到 Vuetify 主題與 document class
 * 在 App.vue 內 setup 一次即可
 */
export const useThemeApply = () => {
  const theme = useTheme()
  const themeStore = useThemeStore()
  const layoutConfigStore = useLayoutConfigStore()
  const coreConfigStore = useConfigStore()

  const resolveMode = () => {
    const m = themeStore.config.mode
    if (m === 'system') {
      return window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
    }
    return m
  }

  const apply = () => {
    const mode = resolveMode()
    // Vuetify 3.7+ 起 `theme.global.name.value = mode` 已棄用,改用 theme.change()
    theme.change(mode)

    // 動態覆蓋 primary 色（兩個 theme 都改）
    const primary = themeStore.config.primary
    const primaryDarken = darken(primary)
    for (const name of ['light', 'dark']) {
      const t = theme.themes.value[name]
      if (!t) continue
      t.colors.primary = primary
      t.colors['primary-darken-1'] = primaryDarken
    }

    // 同步到 @layouts configStore(讓 .layout-content-width-boxed / fluid 切換生效)
    layoutConfigStore.appContentWidth = themeStore.config.contentWidth
    // 永遠強制:垂直 layout、不收合、無 overlay
    layoutConfigStore.isVerticalNavCollapsed = false
    layoutConfigStore.appContentLayoutNav = 'vertical'
    // 同步到 @core configStore(半暗色 sidebar)
    coreConfigStore.isVerticalNavSemiDark = !!themeStore.config.semiDark

    // 樣式:有邊框 → body class
    const html = document.documentElement
    html.classList.toggle('style-bordered', themeStore.config.style === 'bordered')
    html.classList.toggle('style-default', themeStore.config.style !== 'bordered')
    html.classList.toggle('semi-dark', !!themeStore.config.semiDark)
    html.classList.toggle('layout-collapsed', themeStore.config.layout === 'collapsed')
  }

  // 監聽 store + system 主題變化
  watch(() => ({ ...themeStore.config }), apply, { deep: true, immediate: true })

  window.matchMedia?.('(prefers-color-scheme: dark)')
    .addEventListener?.('change', () => {
      if (themeStore.config.mode === 'system') apply()
    })

  return { apply }
}
