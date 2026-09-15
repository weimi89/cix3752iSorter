<script setup>
import { useRoute } from 'vue-router'
import { layoutConfig } from '@layouts'
import { useLayoutConfigStore } from '@layouts/stores/config'
import { injectionKeyIsVerticalNavHovered } from '@layouts/symbols'
import {
  getComputedNavLinkToProp,
  getDynamicI18nProps,
  isNavLinkActive,
} from '@layouts/utils'

const props = defineProps({
  item: {
    type: null,
    required: true,
  },
})

const configStore = useLayoutConfigStore()

const isVerticalNavHovered = inject(injectionKeyIsVerticalNavHovered, ref(false))
const hideTitleAndBadge = configStore.isVerticalNavMini(isVerticalNavHovered)

// vue-router reactive route
const route = useRoute()

const isActive = computed(() => isNavLinkActive(props.item, route))

const linkProps = computed(() => getComputedNavLinkToProp.value(props.item))

const titleI18nProps = computed(() => getDynamicI18nProps(props.item.title, 'span'))
const badgeI18nProps = computed(() => getDynamicI18nProps(props.item.badgeContent, 'span'))
</script>

<template>
  <li
    class="nav-link"
    :class="{ disabled: item.disable }"
  >
    <Component
      :is="item?.to ? 'RouterLink' : 'a'"
      v-bind="linkProps"
      :class="{ 'router-link-active router-link-exact-active': isActive }"
    >
      <Component
        :is="layoutConfig.app.iconRenderer || 'div'"
        v-bind="item.icon || layoutConfig.verticalNav.defaultNavItemIconProps"
        class="nav-item-icon"
      />
      <Transition name="transition-slide-x">
        <Component
          :is="layoutConfig.app.i18n.enable ? 'i18n-t' : 'span'"
          v-show="!hideTitleAndBadge"
          class="nav-item-title"
          v-bind="titleI18nProps"
        >
          {{ item.title }}
        </Component>
      </Transition>

      <Transition
        v-if="item.badgeContent"
        name="transition-slide-x"
      >
        <Component
          :is="layoutConfig.app.i18n.enable ? 'i18n-t' : 'span'"
          v-show="!hideTitleAndBadge"
          class="nav-item-badge"
          :class="item.badgeClass"
          v-bind="badgeI18nProps"
        >
          {{ item.badgeContent }}
        </Component>
      </Transition>
    </Component>
  </li>
</template>

<style lang="scss">
.layout-vertical-nav {
  .nav-link a {
    display: flex;
    align-items: center;
  }
}
</style>
