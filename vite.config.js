import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import vuetify from 'vite-plugin-vuetify'
import AutoImport from 'unplugin-auto-import/vite'
import Components from 'unplugin-vue-components/vite'
import { fileURLToPath, URL } from 'node:url'

// 瀏覽器開發：Vite 代理 /api 與 /events 到後端；桌面模式（tauri dev）前端會直接打後端位址，不經代理
const BACKEND = process.env.CIX_BACKEND || 'http://127.0.0.1:18090'
const host = process.env.TAURI_DEV_HOST

export default defineConfig({
  clearScreen: false,
  plugins: [
    vue(),
    // configFile 只餵斷點（見該檔說明），沒接的話 Vuetify 的 d-md-*／v-col-* 會用原廠 600/840/1145 切
    vuetify({ autoImport: true, styles: { configFile: 'src/styles/vuetify-settings.scss' } }),
    AutoImport({
      imports: ['vue', 'vue-router', '@vueuse/core', 'pinia'],
      dirs: ['./src/@core/utils', './src/@core/composable', './src/composables'],
      vueTemplate: true,
      dts: 'src/auto-imports.d.ts',
      eslintrc: { enabled: false },
    }),
    Components({ dirs: ['src/components'], dts: 'src/components.d.ts' }),
  ],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
      '@core': fileURLToPath(new URL('./src/@core', import.meta.url)),
      '@core-scss': fileURLToPath(new URL('./src/styles/@core', import.meta.url)),
      '@layouts': fileURLToPath(new URL('./src/@layouts', import.meta.url)),
      '@styles': fileURLToPath(new URL('./src/styles', import.meta.url)),
      '@configured-variables': fileURLToPath(new URL('./src/styles/variables/_template.scss', import.meta.url)),
      '@images': fileURLToPath(new URL('./src/assets/images', import.meta.url)),
      '@themeConfig': fileURLToPath(new URL('./themeConfig.js', import.meta.url)),
    },
  },
  publicDir: 'public',
  server: {
    port: 5180,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 5181 } : undefined,
    proxy: Object.fromEntries(['/api', '/auth', '/events'].map(p => [p, { target: BACKEND, changeOrigin: false }])),
    // 桌面視窗（WebKit）在 Vite 一起來就載入，第一批模組還在轉換／相依還在打包時會拿到錯誤回應，
    // WebKit 不會重試，整頁就停在「Importing a module script failed」。先把入口與所有頁面暖起來
    warmup: { clientFiles: ['./src/main.js', './src/pages/*.vue', './src/components/*.vue'] },
  },
  // 相依一律在啟動時就打包好，不要等第一個請求才發現（同上，WebKit 撞到重打包會直接失敗）
  optimizeDeps: {
    include: ['vue', 'vue-router', 'pinia', 'vue-i18n', 'vuetify', 'vue3-toastify', 'vue-echarts', 'echarts/core', 'echarts/charts', 'echarts/components', 'echarts/renderers', 'viewerjs', '@vueuse/core', 'qrcode', '@tauri-apps/api/core', '@tauri-apps/plugin-process', '@tauri-apps/plugin-updater'],
  },
  build: {
    // 工控機瀏覽器可能是舊版 Chromium；分揀線現場也會用手機開
    target: 'chrome105',
    outDir: 'dist',
    emptyOutDir: true,
  },
})
