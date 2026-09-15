import { defineStore } from 'pinia'

const STORAGE_KEY = 'cix3752iSorter.themeConfig'

const DEFAULT_CONFIG = {
  primary: '#76C043',      // 乖乖綠（與 cix3752iLabelPrint 同色）
  mode: 'light',           // 固定明亮(主題選項已從 customizer 拿掉)
  style: 'default',        // 固定 default(樣式選項已從 customizer 拿掉)
  semiDark: true,         // 固定半暗色 sidebar(已從 customizer 拿掉開關)
  layout: 'vertical',      // 桌面 App 固定 vertical(布局選項已從 customizer 拿掉)
  contentWidth: 'fluid',   // fluid(寬鬆) / boxed(緊湊),對齊 @layouts ContentWidth
}

const load = () => {
  try {
    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) || 'null') || {}
    // 不支援 'system' 模式,存到這個值一律退回預設(始終 light)
    if (saved.mode === 'system') delete saved.mode
    // contentWidth 只認 @layouts ContentWidth 的 'fluid' / 'boxed';機器上可能存有 'wide' / 'compact'
    if (saved.contentWidth === 'wide') saved.contentWidth = 'fluid'
    if (saved.contentWidth === 'compact') saved.contentWidth = 'boxed'
    return { ...DEFAULT_CONFIG, ...saved }
  } catch {
    return { ...DEFAULT_CONFIG }
  }
}

export const useThemeStore = defineStore('theme', {
  state: () => ({
    config: load(),
  }),
  actions: {
    persist() {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(this.config))
    },
    set(patch) {
      this.config = { ...this.config, ...patch }
      this.persist()
    },
    reset() {
      this.config = { ...DEFAULT_CONFIG }
      this.persist()
    },
  },
})

// 預設主色 preset
export const PRIMARY_PRESETS = [
  { value: '#7367F0', labelKey: 'theme.preset.purple' },
  { value: '#0D9394', labelKey: 'theme.preset.cyan' },
  { value: '#FFB400', labelKey: 'theme.preset.orange' },
  { value: '#FF4C51', labelKey: 'theme.preset.red' },
  { value: '#16B1FF', labelKey: 'theme.preset.blue' },
]

// 推算 darken-1：把色相略暗 8%（Materio 用 #675DD8 對 #7367F0）
export const darken = hex => {
  const n = parseInt(hex.replace('#', ''), 16)
  let r = (n >> 16) & 0xff, g = (n >> 8) & 0xff, b = n & 0xff
  const f = 0.91
  r = Math.round(r * f); g = Math.round(g * f); b = Math.round(b * f)
  return `#${[r, g, b].map(v => v.toString(16).padStart(2, '0')).join('')}`
}
