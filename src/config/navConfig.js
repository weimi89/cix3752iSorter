/** 側欄導覽（@layouts VerticalNav 的扁平陣列格式） */
export const navItems = [
  { heading: 'nav.section.live' },
  { title: 'nav.dashboard', icon: { icon: 'tabler-layout-dashboard' }, to: { name: 'dashboard' } },
  { title: 'nav.stats', icon: { icon: 'tabler-chart-pie' }, to: { name: 'stats' } },
  { title: 'nav.parcels', icon: { icon: 'tabler-packages' }, to: { name: 'parcels' } },

  { heading: 'nav.section.queues' },
  { title: 'nav.printJobs', icon: { icon: 'tabler-printer' }, to: { name: 'print-jobs' } },
  { title: 'nav.reportQueue', icon: { icon: 'tabler-cloud-upload' }, to: { name: 'report-queue' } },
  { title: 'nav.eventLog', icon: { icon: 'tabler-bell-ringing' }, to: { name: 'event-log' } },

  { heading: 'nav.section.settings' },
  { title: 'nav.chutes', icon: { icon: 'tabler-route' }, to: { name: 'chutes' } },
  { title: 'nav.printers', icon: { icon: 'tabler-usb' }, to: { name: 'printers' } },
  { title: 'nav.devices', icon: { icon: 'tabler-settings' }, to: { name: 'devices' } },
  { title: 'nav.ir', icon: { icon: 'tabler-viewfinder' }, to: { name: 'ir' } },
]
