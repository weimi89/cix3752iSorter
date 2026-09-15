import { createRouter, createWebHashHistory } from 'vue-router'

// 頁面動態載入：現場手機走 Wi-Fi 開網頁，只載該頁需要的部分
const routes = [
  { path: '/', name: 'dashboard', component: () => import('@/pages/DashboardPage.vue'), meta: { title: 'nav.dashboard' } },
  { path: '/parcels', name: 'parcels', component: () => import('@/pages/ParcelsPage.vue'), meta: { title: 'nav.parcels' } },
  { path: '/print-jobs', name: 'print-jobs', component: () => import('@/pages/PrintJobsPage.vue'), meta: { title: 'nav.printJobs' } },
  { path: '/report-queue', name: 'report-queue', component: () => import('@/pages/ReportQueuePage.vue'), meta: { title: 'nav.reportQueue' } },
  { path: '/event-log', name: 'event-log', component: () => import('@/pages/EventLogPage.vue'), meta: { title: 'nav.eventLog' } },
  { path: '/chutes', name: 'chutes', component: () => import('@/pages/ChutesPage.vue'), meta: { title: 'nav.chutes' } },
  { path: '/printers', name: 'printers', component: () => import('@/pages/PrinterSettingsPage.vue'), meta: { title: 'nav.printers' } },
  { path: '/devices', name: 'devices', component: () => import('@/pages/DeviceSettingsPage.vue'), meta: { title: 'nav.devices' } },
  { path: '/switch-demo', name: 'switch-demo', component: () => import('@/pages/SwitchDemoPage.vue') },
  { path: '/:pathMatch(.*)*', redirect: '/' },
]

const router = createRouter({
  history: createWebHashHistory(),
  routes,
})

export default router
