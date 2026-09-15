import { layoutConfig } from '@layouts/config'
import { cookieRef, useLayoutConfigStore } from '@layouts/stores/config'
import { _setDirAttr } from '@layouts/utils'

// 🔌 Plugin
export const createLayouts = userConfig => {
  return () => {
    const configStore = useLayoutConfigStore()

    // Non reactive Values
    layoutConfig.app.namespace = userConfig.app?.namespace ?? layoutConfig.app.namespace
    layoutConfig.app.title = userConfig.app?.title ?? layoutConfig.app.title
    layoutConfig.app.logo = userConfig.app?.logo ?? layoutConfig.app.logo
    layoutConfig.app.overlayNavFromBreakpoint = userConfig.app?.overlayNavFromBreakpoint ?? layoutConfig.app.overlayNavFromBreakpoint
    layoutConfig.app.i18n.enable = userConfig.app?.i18n?.enable ?? layoutConfig.app.i18n.enable
    layoutConfig.app.iconRenderer = userConfig.app?.iconRenderer ?? layoutConfig.app.iconRenderer
    layoutConfig.verticalNav.defaultNavItemIconProps = userConfig.verticalNav?.defaultNavItemIconProps ?? layoutConfig.verticalNav.defaultNavItemIconProps
    layoutConfig.icons.chevronDown = userConfig.icons?.chevronDown ?? layoutConfig.icons.chevronDown
    layoutConfig.icons.chevronRight = userConfig.icons?.chevronRight ?? layoutConfig.icons.chevronRight
    layoutConfig.icons.close = userConfig.icons?.close ?? layoutConfig.icons.close
    layoutConfig.icons.verticalNavPinned = userConfig.icons?.verticalNavPinned ?? layoutConfig.icons.verticalNavPinned
    layoutConfig.icons.verticalNavUnPinned = userConfig.icons?.verticalNavUnPinned ?? layoutConfig.icons.verticalNavUnPinned
    layoutConfig.icons.sectionTitlePlaceholder = userConfig.icons?.sectionTitlePlaceholder ?? layoutConfig.icons.sectionTitlePlaceholder

    // Reactive Values (Store)
    configStore.$patch({
      appContentLayoutNav: cookieRef('appContentLayoutNav', userConfig.app?.contentLayoutNav ?? layoutConfig.app.contentLayoutNav).value,
      appContentWidth: cookieRef('appContentWidth', userConfig.app?.contentWidth ?? layoutConfig.app.contentWidth).value,
      footerType: cookieRef('footerType', userConfig.footer?.type ?? layoutConfig.footer.type).value,
      navbarType: cookieRef('navbarType', userConfig.navbar?.type ?? layoutConfig.navbar.type).value,
      isNavbarBlurEnabled: cookieRef('isNavbarBlurEnabled', userConfig.navbar?.navbarBlur ?? layoutConfig.navbar.navbarBlur).value,
      isVerticalNavCollapsed: cookieRef('isVerticalNavCollapsed', userConfig.verticalNav?.isVerticalNavCollapsed ?? layoutConfig.verticalNav.isVerticalNavCollapsed).value,

      // isLessThanOverlayNavBreakpoint: false,
      horizontalNavType: cookieRef('horizontalNavType', userConfig.horizontalNav?.type ?? layoutConfig.horizontalNav.type).value,
    })
  }
}
export * from './components'
export { layoutConfig }
