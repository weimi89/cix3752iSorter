import VueToastify from 'vue3-toastify'
import 'vue3-toastify/dist/index.css'
import { h } from 'vue'
import { Icon } from '@iconify/vue'

// toast 內建的語氣圖示是實心色塊(白勾疊在綠圓上),跟 App 其他提示的線條圖示對不起來,
// 短訊息左邊也等於多了一塊色塊。改用與 VAlert 同一組 tabler 線條圖示。
const TOAST_ICONS = {
  success: 'tabler:circle-check',
  error: 'tabler:alert-circle',
  warning: 'tabler:alert-triangle',
  info: 'tabler:info-circle',
}

export default function (app) {
  app.use(VueToastify, {
    autoClose: 3000,
    position: 'bottom-right',
    newestOnTop: true,
    // 回 false 代表該語氣不顯示圖示(預設 type 沒有對應圖示,本來就不該硬塞一個)
    icon: ({ type }) => (TOAST_ICONS[type] ? h(Icon, { icon: TOAST_ICONS[type], width: 20, height: 20 }) : false),
  })
}
