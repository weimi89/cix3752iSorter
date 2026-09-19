import { createRouter, createWebHashHistory } from 'vue-router'
import { isTauriRuntime } from '@/api/runtime'
import { useWebAuth } from '@/composables/useWebAuth'

// 頁面動態載入：現場手機走 Wi-Fi 開網頁，只載該頁需要的部分
const routes = [
  { path: '/', name: 'dashboard', component: () => import('@/pages/DashboardPage.vue'), meta: { title: 'nav.dashboard' } },
  { path: '/stats', name: 'stats', component: () => import('@/pages/StatsPage.vue'), meta: { title: 'nav.stats' } },
  { path: '/abnormal', name: 'abnormal', component: () => import('@/pages/AbnormalPage.vue'), meta: { title: 'nav.abnormal' } },
  { path: '/parcels', name: 'parcels', component: () => import('@/pages/ParcelsPage.vue'), meta: { title: 'nav.parcels' } },
  { path: '/print-jobs', name: 'print-jobs', component: () => import('@/pages/PrintJobsPage.vue'), meta: { title: 'nav.printJobs' } },
  { path: '/report-queue', name: 'report-queue', component: () => import('@/pages/ReportQueuePage.vue'), meta: { title: 'nav.reportQueue' } },
  { path: '/event-log', name: 'event-log', component: () => import('@/pages/EventLogPage.vue'), meta: { title: 'nav.eventLog' } },
  { path: '/chutes', name: 'chutes', component: () => import('@/pages/ChutesPage.vue'), meta: { title: 'nav.chutes' } },
  { path: '/printers', name: 'printers', component: () => import('@/pages/PrinterSettingsPage.vue'), meta: { title: 'nav.printers' } },
  { path: '/devices', name: 'devices', component: () => import('@/pages/DeviceSettingsPage.vue'), meta: { title: 'nav.devices' } },
  { path: '/ir', name: 'ir', component: () => import('@/pages/IrCheckPage.vue'), meta: { title: 'nav.ir' } },
  { path: '/switch-demo', name: 'switch-demo', component: () => import('@/pages/SwitchDemoPage.vue') },
  { path: '/login', name: 'login', component: () => import('@/pages/LoginPage.vue'), meta: { public: true } },
  { path: '/:pathMatch(.*)*', redirect: '/' },
]

const router = createRouter({
  history: createWebHashHistory(),
  routes,
})

// 網頁版的登入守衛。
// 桌面視窗與內網來源都不會停在這裡 —— 後端把它們直接放行，refresh() 回報 authenticated 後照常前往目的地。
router.beforeEach(async to => {
  if (isTauriRuntime || to.meta.public) return true

  const { authenticated, checked, refresh } = useWebAuth()

  // 每次開頁都重查一次太浪費，但首次進站一定要問過後端才知道自己算不算內網
  if (!checked.value) await refresh()
  if (authenticated.value) return true

  // 再確認一次：session 可能在別的分頁剛登入
  if (await refresh()) return true

  return { name: 'login', query: to.fullPath === '/' ? {} : { redirect: to.fullPath } }
})

export default router
