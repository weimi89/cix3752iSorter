import { layoutConfig } from '@layouts/config'
import { AppContentLayoutNav } from '@layouts/enums'
import { useLayoutConfigStore } from '@layouts/stores/config'

export const openGroups = ref([])

/**
 * 返回用於 RouterLink 的 nav 連結屬性(vue-router 版本)
 * @param {Object, String} item nav-link 設定
 */
export const getComputedNavLinkToProp = computed(() => link => {
  const props = {
    target: link.target,
    rel: link.rel,
  }

  // link.to 直接交給 RouterLink (可為 string 或 RouteLocationRaw)
  if (link?.to) {
    props.to = link.to
  } else if (link?.href) {
    // 外部連結走 a tag 用 href
    props.href = link.href
  }

  return props
})

/**
 * 返回導航鏈接的路由名稱
 * 如果鏈接是字符串，它將假定它是路由名稱
 * 如果鏈接是對象，它將解析對象並返回該鏈接
 * @param {Object, String} link 導航鏈接對象/字符串
 */
export const resolveNavLinkRouteName = (link) => {
  if (!link.to)
    return null
  if (typeof link.to === 'string')
    return link.to

  return link.to?.name || null
}

/**
 * 檢查 nav-link 是否處於活動狀態(vue-router 版本,需在 component setup 中呼叫)
 * @param {object} link nav-link 對象
 * @param {object} currentRoute vue-router 當前 route(useRoute() 的值)
 */
export const isNavLinkActive = (link, currentRoute = null) => {
  // 沒提供 currentRoute 時(舊呼叫方式),fallback 回 false — caller 應傳入
  if (!currentRoute) return false

  const targetName = resolveNavLinkRouteName(link)
  if (!targetName) return false

  if (currentRoute.name === targetName) return true

  const extraRoutes = link?.extra
  if (extraRoutes) {
    const extraRoutesArray = Array.isArray(extraRoutes) ? extraRoutes : [extraRoutes]
    return extraRoutesArray.some(extra => currentRoute.name === extra)
  }

  return false
}

/**
 * 檢查 nav 組是否處於活動狀態
 * @param {Array} children 組子元素
 * @param {object} currentRoute vue-router 當前 route
 */
export const isNavGroupActive = (children, currentRoute = null) => children.some(child => {
  if ('children' in child) {
    return isNavGroupActive(child.children, currentRoute)
  }
  return isNavLinkActive(child, currentRoute)
})

/**
 * 根據方向更改 `dir` 屬性
 * @param dir 'ltr' | 'rtl'
 */
export const _setDirAttr = dir => {
  // Check if document exists for SSR
  if (typeof document !== 'undefined')
    document.documentElement.setAttribute('dir', dir)
}

/**
 * 根據 i18n 插件是否啟用返回動態 i18n 屬性
 * @param key i18n 翻譯鍵
 * @param tag 用於包裝翻譯的標籤
 */
export const getDynamicI18nProps = (key, tag = 'span') => {
  if (!layoutConfig.app.i18n.enable)
    return {}

  return {
    keypath: key,
    tag,
    scope: 'global',
  }
}
export const switchToVerticalNavOnLtOverlayNavBreakpoint = () => {
  const configStore = useLayoutConfigStore()

  /*
    ℹ️ 這是標誌，將在從 mdAndDown 窗口寬度切換到 lgAndUp 時保持需要呈現的導航類型

    要求：當我們導航設置為 `horizontal` 並且我們命中 `mdAndDown` 斷點時，導航類型應該更改為 `vertical` 導航
    現在，如果我們從 `mdAndDown` 返回到 `lgAndUp`，在大設備中之前的導航類型是什麼，我們將如何知道？

    讓我們將 `appContentLayoutNav` 的值分配為 `lgAndUpNav` 的默認值。為什麼呢 🤔？
      如果在 lgAndUp 中查看模板
        我們將 `appContentLayoutNav` 的值分配給 `lgAndUpNav`，因為在這一點上，這兩個常數是相同的
        因此，對於 `lgAndUpNav`，它將從主題配置文件中取得值
      否則
        它將始終顯示垂直導航，如果用戶增加窗口寬度，它將回退到 `appContentLayoutNav` 的值
        但 `appContentLayoutNav` 的值將是在主題配置文件中設置的值
  */
  const lgAndUpNav = ref(configStore.appContentLayoutNav)


  /*
    有可能我們手動從垂直導航切換到水平導航，反之亦然，在 `lgAndUp` 窗口中切換
    因此，當用戶從 `mdAndDown` 返回到 `lgAndUp` 時，我們可以設置更新的導航類型
    為此，我們需要在屏幕是 `lgAndUp` 時更新 `lgAndUpNav` 的值
  */
  watch(() => configStore.appContentLayoutNav, value => {
    if (!configStore.isLessThanOverlayNavBreakpoint)
      lgAndUpNav.value = value
  })

  /*
    這是佈局切換部分
    如果是 `mdAndDown` => 我們將始終使用垂直導航，無論先前的導航類型是什麼
    或者如果是 `lgAndUp`，我們需要切換回 `lgAndUp` 導航類型。為此，我們將跟踪屬性 `lgAndUpNav`
  */
  watch(() => configStore.isLessThanOverlayNavBreakpoint, val => {
    configStore.appContentLayoutNav = val ? AppContentLayoutNav.Vertical : lgAndUpNav.value
  }, { immediate: true })
}

/**
 * 將 Hex 顏色轉換為 RGB
 * @param hex
 */
export const hexToRgb = hex => {
  // 將簡寫形式（例如"03F"）擴展為完整形式（例如"0033FF"）
  const shorthandRegex = /^#?([a-f\d])([a-f\d])([a-f\d])$/i

  hex = hex.replace(shorthandRegex, (m, r, g, b) => {
    return r + r + g + g + b + b
  })

  const result = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})$/i.exec(hex)

  return result ? `${Number.parseInt(result[1], 16)},${Number.parseInt(result[2], 16)},${Number.parseInt(result[3], 16)}` : null
}

/**
 * RGBA 顏色轉換為 Hex 顏色，帶/不帶不透明度
 */
export const rgbaToHex = (rgba, forceRemoveAlpha = false) => {
  return (`#${rgba
    .replace(/^rgba?\(|\s+|\)$/g, '') // 獲取 rgba / rgb 字符串值
    .split(',') // 在“,”處拆分它們
    .filter((string, index) => !forceRemoveAlpha || index !== 3)
    .map(string => Number.parseFloat(string)) // 將它們轉換為數字
    .map((number, index) => (index === 3 ? Math.round(number * 255) : number)) // 將 alpha 轉換為 255 數字
    .map(number => number.toString(16)) // 將數字轉換為十六進制
    .map(string => (string.length === 1 ? `0${string}` : string)) // 長度為 1 時添加 0
    .join('')}`)
}
