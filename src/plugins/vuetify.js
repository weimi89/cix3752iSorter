import { createVuetify } from 'vuetify'
import { aliases } from 'vuetify/iconsets/mdi'
import { createVueI18nAdapter } from 'vuetify/locale/adapters/vue-i18n'
import { h } from 'vue'
import { useI18n } from 'vue-i18n'
import { Icon, addCollection } from '@iconify/vue'
import offlineIcons from './icons-offline.json'

import { getI18n } from '@/plugins/i18n'
import defaults from './vuetify-defaults'

// 把專案實際用到的圖示打包進來（見 scripts/build-icon-subset.mjs）。
// 少了這段,@iconify/vue 找不到本地資料時會去 api.iconify.design 線上抓 ——
// 外網一斷,畫面上每個圖示都變成空白,連導覽列有哪些按鈕都看不出來。
// 中介機為了打雲端 API 本來就有外網,所以現場一直沒踩到,但這個 App 的整套
// 網路偵測設計就是為了撐過斷網,UI 不該在那時候先瞎掉。
for (const collection of Object.values(offlineIcons)) addCollection(collection)

// 正規化 Materio 的 dash 寫法（tabler-xxx / mdi-xxx）成 iconify 的 prefix:name 格式
const normalizeIconName = name => {
  if (!name || typeof name !== 'string') return name
  if (name.includes(':')) return name
  if (name.startsWith('tabler-')) return `tabler:${name.slice(7)}`
  if (name.startsWith('mdi-')) return `mdi:${name.slice(4)}`
  if (name.startsWith('fa6-solid-')) return `fa6-solid:${name.slice(10)}`
  return name
}

const iconify = {
  component: props => h(Icon, { ...props, icon: normalizeIconName(props.icon) }),
}

const lightTheme = {
  dark: false,
  colors: {
    primary: '#7367F0',
    'on-primary': '#fff',
    'primary-darken-1': '#675DD8',
    secondary: '#808390',
    'on-secondary': '#fff',
    'secondary-darken-1': '#737682',
    success: '#28C76F',
    'on-success': '#fff',
    'success-darken-1': '#24B364',
    info: '#00BAD1',
    'on-info': '#fff',
    'info-darken-1': '#00A7BC',
    warning: '#FF9F43',
    'on-warning': '#fff',
    'warning-darken-1': '#E68F3C',
    error: '#FF4C51',
    'on-error': '#fff',
    'error-darken-1': '#E64449',
    background: '#F8F7FA',
    'on-background': '#2F2B3D',
    surface: '#fff',
    'on-surface': '#2F2B3D',
    'grey-50': '#FAFAFA',
    'grey-100': '#F5F5F5',
    'grey-200': '#EEEEEE',
    'grey-300': '#E0E0E0',
    'grey-400': '#BDBDBD',
    'grey-500': '#9E9E9E',
    'grey-600': '#757575',
    'grey-700': '#616161',
    'grey-800': '#424242',
    'grey-900': '#212121',
  },
  variables: {
    'border-color': '#2F2B3D',
    'border-opacity': 0.12,
    'high-emphasis-opacity': 0.9,
    'medium-emphasis-opacity': 0.7,
    'hover-opacity': 0.06,
    'focus-opacity': 0.1,
    'selected-opacity': 0.08,
    'activated-opacity': 0.16,
    'pressed-opacity': 0.14,
    'shadow-key-umbra-color': '#2F2B3D',
    'shadow-xs-opacity': 0.10,
    'shadow-sm-opacity': 0.12,
    'shadow-md-opacity': 0.14,
    'shadow-lg-opacity': 0.16,
    'shadow-xl-opacity': 0.18,
    'track-bg': '#F1F0F2',
  },
}

const darkTheme = {
  dark: true,
  colors: {
    ...lightTheme.colors,
    background: '#25293C',
    'on-background': '#E1DEF5',
    surface: '#2F3349',
    'on-surface': '#E1DEF5',
  },
  variables: {
    ...lightTheme.variables,
    'border-color': '#E1DEF5',
    'shadow-key-umbra-color': '#131120',
    'track-bg': '#3A3F57',
  },
}

export default createVuetify({
  // 讓 Vuetify 內建元件標籤($vuetify.input.clear / $vuetify.close 等)走 vue-i18n,
  // 訊息已在 plugins/i18n 併入各語系的 $vuetify(內建 zhHant / vi)。
  // 未接時 Vuetify 用自己那套(無 zh-Hant 訊息)→ 每個 clearable input / dialog 都噴
  // 「Translation key not found」警告,dev 模式下每筆警告序列化整棵元件樹(含 devtools-api 巨物)
  // → GB 級 log 灌爆、切頁與操作嚴重卡頓。
  locale: {
    adapter: createVueI18nAdapter({ i18n: getI18n(), useI18n }),
  },
  theme: {
    defaultTheme: 'light',
    themes: { light: lightTheme, dark: darkTheme },
  },
  // Vuetify 4 把預設斷點縮小(md 840 / lg 1145 / xl 1545),側欄收合與 d-md-* 切換點會整個位移;
  // 鎖回 v3 預設維持現行版面行為(SCSS 端 $grid-breakpoints 另在 styles/variables/_vuetify.scss)
  display: {
    thresholds: { md: 960, lg: 1280, xl: 1920, xxl: 2560 },
  },
  icons: {
    defaultSet: 'iconify',
    // mdi 那組 alias 的語氣圖示是實心色塊(error 是填滿的圓 + 白色 X),在提示訊息裡
    // 看起來像左邊多了一顆有底色的按鈕,把整條短訊息撐開。全站其他圖示都是 tabler 線條版,
    // 這四個跟著改成線條版才一致 —— 其餘 alias(展開、排序、分頁…)維持 mdi 不動。
    aliases: {
      ...aliases,
      success: 'tabler-circle-check',
      info: 'tabler-info-circle',
      warning: 'tabler-alert-triangle',
      error: 'tabler-alert-circle',
    },
    sets: { iconify },
  },
  defaults,
})
