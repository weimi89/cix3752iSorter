<script setup>
import { useStorage } from '@vueuse/core'
import { OverlayScrollbarsComponent } from 'overlayscrollbars-vue'
import { useTheme } from 'vuetify'
import {
  staticPrimaryColor,
  staticPrimaryDarkenColor,
} from '@/plugins/vuetify/theme'
import {
  Layout,
  Skins,
  Theme,
} from '@core/enums'
import { useConfigStore } from '@core/stores/config'
import horizontalLight from '@images/customizer-icons/horizontal-light.svg'
import {
  AppContentLayoutNav,
  ContentWidth,
} from '@layouts/enums'
import {
  cookieRef,
  namespaceConfig,
} from '@layouts/stores/config'
import { themeConfig } from '@themeConfig'
import borderSkin from '@images/customizer-icons/border-light.svg'
import collapsed from '@images/customizer-icons/collapsed-light.svg'
import compact from '@images/customizer-icons/compact-light.svg'
import defaultSkin from '@images/customizer-icons/default-light.svg'
import wideSvg from '@images/customizer-icons/wide-light.svg'

// 定義 props 接收 v-model
const props = defineProps({
  modelValue: {
    type: Boolean,
    default: false,
  },
})

// 定義 emits，將 v-model 的事件傳遞給父組件
const emit = defineEmits()

const isNavDrawerOpen = ref(props.modelValue)
const configStore = useConfigStore()
const vuetifyTheme = useTheme()

const colors = [
  {
    main: staticPrimaryColor,
    darken: staticPrimaryDarkenColor,
  },
  {
    main: '#0D9394',
    darken: '#0C8485',
  },
  {
    main: '#FFB400',
    darken: '#E6A200',
  },
  {
    main: '#FF4C51',
    darken: '#E64449',
  },
  {
    main: '#16B1FF',
    darken: '#149FE6',
  },
]

const customPrimaryColor = ref('#663131')

// 監控 prop 的變化並更新本地狀態
watch(() => props.modelValue, (newValue) => {
  isNavDrawerOpen.value = newValue
})

// 監控本地狀態的變化並發出更新事件
watch(isNavDrawerOpen, (newValue) => {
  emit('update:modelValue', newValue)
})

watch(() => configStore.theme, () => {
  const cookiePrimaryColor = cookieRef(`${ vuetifyTheme.name.value }ThemePrimaryColor`, null).value
  if (cookiePrimaryColor && !colors.some(color => color.main === cookiePrimaryColor))
    customPrimaryColor.value = cookiePrimaryColor
}, { immediate: true })

const setPrimaryColor = useDebounceFn(color => {
  vuetifyTheme.themes.value[vuetifyTheme.name.value].colors.primary = color.main
  vuetifyTheme.themes.value[vuetifyTheme.name.value].colors['primary-darken-1'] = color.darken
  cookieRef(`${ vuetifyTheme.name.value }ThemePrimaryColor`, null).value = color.main
  cookieRef(`${ vuetifyTheme.name.value }ThemePrimaryDarkenColor`, null).value = color.darken
  useStorage(namespaceConfig('initial-loader-color'), null).value = color.main
}, 100)

const themeMode = computed(() => {
  return [
    {
      bgImage: 'tabler-sun',
      value: Theme.Light,
      label: '明亮模式',
    },
    {
      bgImage: 'tabler-moon-stars',
      value: Theme.Dark,
      label: '暗黑模式',
    },
    {
      bgImage: 'tabler-device-desktop-analytics',
      value: Theme.System,
      label: '系統模式',
    },
  ]
})

const themeSkin = computed(() => {
  return [
    {
      bgImage: defaultSkin,
      value: Skins.Default,
      label: '預設',
    },
    {
      bgImage: borderSkin,
      value: Skins.Bordered,
      label: '有邊框',
    },
  ]
})

const currentLayout = ref(configStore.isVerticalNavCollapsed ? 'collapsed' : configStore.appContentLayoutNav)

const layouts = computed(() => {
  return [
    {
      bgImage: defaultSkin,
      value: Layout.Vertical,
      label: '垂直',
    },
    {
      bgImage: collapsed,
      value: Layout.Collapsed,
      label: '收合',
    },
    {
      bgImage: horizontalLight,
      value: Layout.Horizontal,
      label: '水平',
    },
  ]
})

watch(currentLayout, () => {
  if (currentLayout.value === 'collapsed') {
    configStore.isVerticalNavCollapsed = true
    configStore.appContentLayoutNav = AppContentLayoutNav.Vertical
  } else {
    configStore.isVerticalNavCollapsed = false
    configStore.appContentLayoutNav = currentLayout.value
  }
})
watch(() => configStore.isVerticalNavCollapsed, () => {
  currentLayout.value = configStore.isVerticalNavCollapsed ? 'collapsed' : configStore.appContentLayoutNav
})

const contentWidth = computed(() => {
  return [
    {
      bgImage: compact,
      value: ContentWidth.Boxed,
      label: '緊湊',
    },
    {
      bgImage: wideSvg,
      value: ContentWidth.Fluid,
      label: '寬鬆',
    },
  ]
})

const isCookieHasAnyValue = ref(false)

watch([
  () => vuetifyTheme.current.value.colors.primary,
  configStore.$state,
], () => {
  const initialConfigValue = [
    staticPrimaryColor,
    staticPrimaryColor,
    themeConfig.app.theme,
    themeConfig.app.skin,
    themeConfig.verticalNav.isVerticalNavSemiDark,
    themeConfig.verticalNav.isVerticalNavCollapsed,
    themeConfig.app.contentWidth,
    themeConfig.app.contentLayoutNav,
  ]

  const themeConfigValue = [
    vuetifyTheme.themes.value.light.colors.primary,
    vuetifyTheme.themes.value.dark.colors.primary,
    configStore.theme,
    configStore.skin,
    configStore.isVerticalNavSemiDark,
    configStore.isVerticalNavCollapsed,
    configStore.appContentWidth,
    configStore.appContentLayoutNav,
  ]

  isCookieHasAnyValue.value = JSON.stringify(themeConfigValue) !== JSON.stringify(initialConfigValue)
}, {
  deep: true,
  immediate: true,
})

const resetCustomizer = async () => {
  if (isCookieHasAnyValue.value) {
    vuetifyTheme.themes.value.light.colors.primary = staticPrimaryColor
    vuetifyTheme.themes.value.dark.colors.primary = staticPrimaryColor
    vuetifyTheme.themes.value.light.colors['primary-darken-1'] = staticPrimaryDarkenColor
    vuetifyTheme.themes.value.dark.colors['primary-darken-1'] = staticPrimaryDarkenColor
    configStore.theme = themeConfig.app.theme
    configStore.skin = themeConfig.app.skin
    configStore.isVerticalNavSemiDark = themeConfig.verticalNav.isVerticalNavSemiDark
    configStore.appContentLayoutNav = themeConfig.app.contentLayoutNav
    configStore.appContentWidth = themeConfig.app.contentWidth
    configStore.isVerticalNavCollapsed = themeConfig.verticalNav.isVerticalNavCollapsed
    useStorage(namespaceConfig('initial-loader-color'), null).value = staticPrimaryColor
    currentLayout.value = themeConfig.app.contentLayoutNav
    cookieRef('lightThemePrimaryColor', null).value = null
    cookieRef('darkThemePrimaryColor', null).value = null
    cookieRef('lightThemePrimaryDarkenColor', null).value = null
    cookieRef('darkThemePrimaryDarkenColor', null).value = null
    await nextTick()
    isCookieHasAnyValue.value = false
    customPrimaryColor.value = '#ffffff'
  }
}
</script>

<template>
  <VNavigationDrawer
    v-model="isNavDrawerOpen"
    temporary
    touchless
    border="none"
    location="end"
    width="400"
    elevation="3"
    :scrim="false"
    class="app-customizer"
  >
    <!-- 👉 Header -->
    <div class="customizer-heading d-flex align-center justify-space-between">
      <div>
        <h6 class="text-title-large">
          主题定制
        </h6>
        <p class="text-body-medium mb-0">
          即時自訂和預覽
        </p>
      </div>
      <div class="d-flex align-center gap-1">
        <VBtn
          icon
          variant="text"
          size="small"
          color="medium-emphasis"
          @click="resetCustomizer"
        >
          <VBadge
            v-show="isCookieHasAnyValue"
            dot
            color="error"
            offset-x="-29"
            offset-y="-14"
          />

          <VIcon
            size="24"
            color="high-emphasis"
            icon="tabler-refresh"
          />
        </VBtn>

        <VBtn
          icon
          variant="text"
          color="medium-emphasis"
          size="small"
          @click="isNavDrawerOpen = false"
        >
          <VIcon
            icon="tabler-x"
            color="high-emphasis"
            size="24"
          />
        </VBtn>
      </div>
    </div>

    <VDivider />

    <OverlayScrollbarsComponent
      element="ul"
      :options="{ overflow: { x: 'hidden' }, scrollbars: { autoHide: 'leave', autoHideDelay: 200 } }"
      defer
    >
      <!-- SECTION 主题 -->
      <CustomizerSection
        title="主题"
        :divider="false"
      >
        <!-- 👉 主色 -->
        <div class="d-flex flex-column gap-2">
          <h6 class="text-title-large">
            主色
          </h6>

          <div
            class="d-flex app-customizer-primary-colors"
            style="column-gap: 0.75rem; margin-block-start: 2px;"
          >
            <div
              v-for="color in colors"
              :key="color.main"
              style="
            border-radius: 0.375rem;
            outline: 1px solid rgba(var(--v-border-color), var(--v-border-opacity));
            padding-block: 0.5rem;
            padding-inline: 0.625rem;"
              class="primary-color-wrapper cursor-pointer"
              :class="vuetifyTheme.current.value.colors.primary === color.main ? 'active' : ''"
              :style="vuetifyTheme.current.value.colors.primary === color.main ? `outline-color: ${color.main}; outline-width:2px;` : `--v-color:${color.main}`"
              @click="setPrimaryColor(color)"
            >
              <div
                style="border-radius: 0.375rem;block-size: 2.125rem; inline-size: 1.8938rem;"
                :style="{ backgroundColor: color.main }"
              />
            </div>

            <div
              class="primary-color-wrapper cursor-pointer d-flex align-center"
              style="
            border-radius: 0.375rem;
            outline: 1px solid rgba(var(--v-border-color), var(--v-border-opacity));
            padding-block: 0.5rem;
            padding-inline: 0.625rem;"
              :class="vuetifyTheme.current.value.colors.primary === customPrimaryColor ? 'active' : ''"
              :style="vuetifyTheme.current.value.colors.primary === customPrimaryColor ? `outline-color: ${customPrimaryColor}; outline-width:2px;` : ''"
            >
              <VBtn
                icon
                size="30"
                :color="vuetifyTheme.current.value.colors.primary === customPrimaryColor ? customPrimaryColor : $vuetify.theme.current.dark ? '#8692d029' : '#4b465c29'"
                variant="flat"
                style="border-radius: 0.375rem;"
              >
                <VIcon
                  size="20"
                  icon="tabler-color-picker"
                  :color="vuetifyTheme.current.value.colors.primary === customPrimaryColor ? 'rgb(var(--v-theme-on-primary))' : ''"
                />
              </VBtn>

              <VMenu
                activator="parent"
                :close-on-content-click="false"
              >
                <VList>
                  <VListItem>
                    <VColorPicker
                      v-model="customPrimaryColor"
                      mode="hex"
                      :modes="['hex']"
                      @update:model-value="setPrimaryColor({ main: customPrimaryColor, darken: customPrimaryColor })"
                    />
                  </VListItem>
                </VList>
              </VMenu>
            </div>
          </div>
        </div>

        <!-- 👉 主题 -->
        <div class="d-flex flex-column gap-2">
          <h6 class="text-title-large">
            主题
          </h6>

          <CustomRadiosWithImage
            :key="configStore.theme"
            v-model:selected-radio="configStore.theme"
            :radio-content="themeMode"
            :grid-column="{ cols: '4' }"
            class="customizer-skins"
          >
            <template #label="item">
              <span class="text-sm text-medium-emphasis mt-1">{{ item?.label }}</span>
            </template>

            <template #content="{ item }">
              <div
                class="customizer-skins-icon-wrapper d-flex align-center justify-center py-3 w-full"
                style="min-inline-size: 100%;"
              >
                <VIcon
                  size="30"
                  :icon="item.bgImage"
                  color="high-emphasis"
                />
              </div>
            </template>
          </CustomRadiosWithImage>
        </div>

        <!-- 👉 樣式 -->
        <div class="d-flex flex-column gap-2">
          <h6 class="text-title-large">
            樣式
          </h6>

          <CustomRadiosWithImage
            :key="configStore.skin"
            v-model:selected-radio="configStore.skin"
            :radio-content="themeSkin"
            :grid-column="{ cols: '4' }"
          >
            <template #label="item">
              <span class="text-sm text-medium-emphasis">{{ item?.label }}</span>
            </template>
          </CustomRadiosWithImage>
        </div>

        <!-- 👉 半暗色 -->
        <div
          class="align-center justify-space-between"
          :class="vuetifyTheme.name.value === 'light' && configStore.appContentLayoutNav === AppContentLayoutNav.Vertical ? 'd-flex' : 'd-none'"
        >
          <VLabel
            for="customizer-semi-dark"
            class="text-title-large text-high-emphasis"
          >
            半暗色選單
          </VLabel>

          <div>
            <VSwitch
              id="customizer-semi-dark"
              v-model="configStore.isVerticalNavSemiDark"
              class="ms-2"
            />
          </div>
        </div>
      </CustomizerSection>
      <!-- !SECTION -->

      <!-- SECTION 布局 -->
      <CustomizerSection title="布局">
        <!-- 👉 布局 -->
        <div class="d-flex flex-column gap-2">
          <h6 class="text-base font-weight-medium">
            布局
          </h6>

          <CustomRadiosWithImage
            :key="currentLayout"
            v-model:selected-radio="currentLayout"
            :radio-content="layouts"
            :grid-column="{ cols: '4' }"
          >
            <template #label="item">
              <span class="text-sm text-medium-emphasis">{{ item.label }}</span>
            </template>
          </CustomRadiosWithImage>
        </div>

        <!-- 👉 内容宽度 -->
        <div class="d-flex flex-column gap-2">
          <h6 class="text-base font-weight-medium">
            内容
          </h6>

          <CustomRadiosWithImage
            :key="configStore.appContentWidth"
            v-model:selected-radio="configStore.appContentWidth"
            :radio-content="contentWidth"
            :grid-column="{ cols: '4' }"
          >
            <template #label="item">
              <span class="text-sm text-medium-emphasis">{{ item.label }}</span>
            </template>
          </CustomRadiosWithImage>
        </div>
      </CustomizerSection>
      <!-- !SECTION -->
    </OverlayScrollbarsComponent>
  </VNavigationDrawer>
</template>

<style lang="scss">
@use "@layouts/styles/mixins" as layoutMixins;

.app-customizer {
  &.v-navigation-drawer--temporary:not(.v-navigation-drawer--active) {
    transform: translateX(110%) !important;
  }

  .customizer-section {
    display: flex;
    flex-direction: column;
    padding: 1.5rem;
    gap: 1.5rem;
  }

  .customizer-heading {
    padding-block: 1rem;
    padding-inline: 1.5rem;
  }

  .custom-input-wrapper {
    .v-col {
      padding-inline: 10px;
    }

    .v-label.custom-input {
      border: none;
      color: rgb(var(--v-theme-on-surface));
      outline: 1px solid rgba(var(--v-border-color), var(--v-border-opacity));
    }
  }

  .v-navigation-drawer__content {
    display: flex;
    flex-direction: column;
  }

  .v-label.custom-input.active {
    border-color: transparent;
    outline: 2px solid rgb(var(--v-theme-primary));
  }

  .v-label.custom-input:not(.active):hover {
    border-color: rgba(var(--v-border-color), 0.22);
  }

  .customizer-skins {
    .custom-input.active {
      .customizer-skins-icon-wrapper {
        background-color: rgba(var(--v-global-theme-primary), var(--v-selected-opacity));
      }
    }
  }

  .app-customizer-primary-colors {
    .primary-color-wrapper:not(.active) {
      &:hover {
        outline-color: rgba(var(--v-border-color), 0.22) !important;
      }
    }
  }
}

.app-customizer-toggler {
  position: fixed !important;
  inset-block-start: 20%;
  inset-inline-end: 0;
}
</style>
