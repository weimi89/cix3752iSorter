import { createApp } from 'vue'
import { createPinia } from 'pinia'

import App from './App.vue'
import router from './router'
import vuetify from './plugins/vuetify'
import toastify from './plugins/toastify'
import i18n from './plugins/i18n'
import layouts from './plugins/layouts'
import { initRuntime } from './api/runtime'

// Vuetify 4 layer 順序 + 選擇性 CSS reset，必須排在 vuetify/styles 之前
import './styles/vuetify-layers.css'
import 'vuetify/styles'
import 'overlayscrollbars/overlayscrollbars.css'
import '@core-scss/template/index.scss'
import './styles/main.scss'

// 桌面模式要先拿到本機伺服器位址，否則第一批請求會打到 tauri://localhost 上不存在的路徑
await initRuntime()

const app = createApp(App)
app.use(createPinia())
app.use(router)
i18n(app)
app.use(vuetify)
layouts(app)
toastify(app)
app.mount('#app')
