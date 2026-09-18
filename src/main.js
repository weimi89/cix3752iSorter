import { createApp } from 'vue'
import { createPinia } from 'pinia'

import App from './App.vue'
import router from './router'
import vuetify from './plugins/vuetify'
import toastify from './plugins/toastify'
import i18n from './plugins/i18n'
import layouts from './plugins/layouts'
import { initRuntime } from './api/runtime'
import { setUnauthorizedHandler } from './api/http'
import { installErrorReporting } from './api/errorReport'
import { useWebAuth } from './composables/useWebAuth'

// Vuetify 4 layer 順序 + 選擇性 CSS reset，必須排在 vuetify/styles 之前
import './styles/vuetify-layers.css'
import 'vuetify/styles'
import 'overlayscrollbars/overlayscrollbars.css'
import '@core-scss/template/index.scss'
import './styles/main.scss'

// 桌面模式要先拿到本機伺服器位址，否則第一批請求會打到 tauri://localhost 上不存在的路徑
await initRuntime()

// 資料請求收到 401（session 逾期、或改密碼清掉了 session）時，把人帶回登入頁。
// 少了這段，逾期後畫面會停在原地一直跳「尚未登入」的錯誤，使用者不知道要去哪重新登入。
setUnauthorizedHandler(() => {
  const { markUnauthenticated } = useWebAuth()
  markUnauthenticated()
  if (router.currentRoute.value.name !== 'login') {
    const from = router.currentRoute.value.fullPath
    router.replace({ name: 'login', query: from === '/' ? {} : { redirect: from } })
  }
})

const app = createApp(App)
// 畫面上的 JavaScript 錯誤送回後端記進事件記錄：正式版桌面沒有開發者工具，元件默默不顯示時才有線索
installErrorReporting(app, router)
app.use(createPinia())
app.use(router)
i18n(app)
app.use(vuetify)
layouts(app)
toastify(app)
// 等路由解析完第一頁再掛載：否則登入頁那一瞬間會先套上主版面、發出資料請求，
// 外網來源會收到 401 而被導向登入頁，把原本要回去的頁面（redirect）洗掉
await router.isReady()
app.mount('#app')
