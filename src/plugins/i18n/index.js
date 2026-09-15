import { createI18n } from 'vue-i18n'
import { zhHant as vuetifyZhHant } from 'vuetify/locale'

const STORAGE_KEY = 'app-locale'
export const DEFAULT_LOCALE = 'zh-Hant'
export const AVAILABLE_LOCALES = [
  { code: 'zh-Hant', label: '繁體中文' },
]

const vuetifyLocales = {
  'zh-Hant': vuetifyZhHant,
}

const messages = Object.fromEntries(
  Object.entries(import.meta.glob('./locales/*.json', { eager: true }))
    .map(([key, value]) => {
      const code = key.slice('./locales/'.length, -'.json'.length)

      return [code, { ...value.default, $vuetify: vuetifyLocales[code] }]
    }),
)

function loadInitialLocale() {
  try {
    const saved = localStorage.getItem(STORAGE_KEY)
    if (saved && AVAILABLE_LOCALES.some(l => l.code === saved)) return saved
  } catch {}

  return DEFAULT_LOCALE
}

let _i18n = null
export const getI18n = () => {
  if (_i18n === null) {
    _i18n = createI18n({
      legacy: false,
      locale: loadInitialLocale(),
      fallbackLocale: DEFAULT_LOCALE,
      messages,
      missingWarn: false,
      fallbackWarn: false,
    })
  }

  return _i18n
}

export function persistLocale(code) {
  try {
    localStorage.setItem(STORAGE_KEY, code)
  } catch {}
}

export default function (app) {
  app.use(getI18n())
}
