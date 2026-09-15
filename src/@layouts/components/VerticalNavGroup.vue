<script setup>
import { useRoute } from 'vue-router'
import { layoutConfig } from '@layouts'
import {
  TransitionExpand,
  VerticalNavLink,
} from '@layouts/components'
import { useLayoutConfigStore } from '@layouts/stores/config'
import { injectionKeyIsVerticalNavHovered } from '@layouts/symbols'
import {
  getDynamicI18nProps,
  isNavGroupActive,
  openGroups,
} from '@layouts/utils'

const route = useRoute()

const props = defineProps({
  item: {
    type: null,
    required: true,
  },
})

defineOptions({
  name: 'VerticalNavGroup',
})

const configStore = useLayoutConfigStore()

/*ℹ️ We provided default value `ref(false)` because inject will return `T | undefined`
Docs: https://vuejs.org/api/composition-api-dependency-injection.html#inject
*/
const isVerticalNavHovered = inject(injectionKeyIsVerticalNavHovered, ref(false))

// 明確傳入已 inject 的 hover ref，避免 isVerticalNavMini 內部再次 inject
const hideTitleAndBadge = configStore.isVerticalNavMini(isVerticalNavHovered)
const isGroupActive = ref(false)
const isGroupOpen = ref(false)

const isAnyChildOpen = children => {
  return children.some(child => {
    let result = openGroups.value.includes(child.title)
    if ('children' in child)
      result = isAnyChildOpen(child.children) || result

    return result
  })
}

const collapseChildren = children => {
  children.forEach(child => {
    if ('children' in child)
      collapseChildren(child.children)
    openGroups.value = openGroups.value.filter(group => group !== child.title)
  })
}

// 子項目預先剝除 icon（巢狀 group/link 統一用 default icon）
// computed 確保 children reference 沒變時，子元件 props 不重建，避免 watcher 觸發整棵子樹 re-render
const processedChildren = computed(() => {
  if (!props.item.children) return []

  return props.item.children.map(child => ({ ...child, icon: undefined }))
})

// i18n props 用 computed 鎖定 reference，i18n 未啟用時等同空物件，避免每次 render 重新 new {} 觸發 v-bind diff
const titleI18nProps = computed(() => getDynamicI18nProps(props.item.title, 'span'))
const badgeI18nProps = computed(() => getDynamicI18nProps(props.item.badgeContent, 'span'))

/*Watch for route changes, more specifically route path. Do note that this won't trigger if route's query is updated.

updates isActive & isOpen based on active state of group.
*/
watch(() => route.fullPath, () => {

  const isActive = isNavGroupActive(props.item.children, route)

  // Don't open group if vertical nav is collapsed and window size is more than overlay nav breakpoint
  isGroupOpen.value = isActive && !configStore.isVerticalNavMini(isVerticalNavHovered).value
  isGroupActive.value = isActive
}, { immediate: true })
watch(isGroupOpen, val => {

  // Find group index for adding/removing group from openGroups array
  const grpIndex = openGroups.value.indexOf(props.item.title)

  // update openGroups array for addition/removal of current group

  // If group is opened => Add it to `openGroups` array
  if (val && grpIndex === -1) {
    openGroups.value.push(props.item.title)
  } else if (!val && grpIndex !== -1) {
    openGroups.value.splice(grpIndex, 1)
    collapseChildren(props.item.children)
  }
}, { immediate: true })

/*Watch for openGroups (accordion 行為)

監聽 `.at(-1)` 取代 deep watch — 整支 watcher 只關心「最後一次新增的 group title」，
deep watch 會在所有 group 同時觸發遞迴比對，非常昂貴；改成 shallow 監聽末項變化。
*/
watch(() => openGroups.value.at(-1), () => {

  // Prevent closing recently opened inactive group.
  if (openGroups.value.at(-1) === props.item.title)
    return

  const isActive = isNavGroupActive(props.item.children, route)

  // Goal of this watcher is to close inactive groups. So don't do anything for active groups.
  if (isActive)
    return

  // We won't close group if any of child group is open in current group
  if (isAnyChildOpen(props.item.children))
    return
  isGroupOpen.value = isActive
  isGroupActive.value = isActive
})

// ℹ️ Previously instead of below watcher we were using two individual watcher for `isVerticalNavHovered`, `isVerticalNavCollapsed` & `isLessThanOverlayNavBreakpoint`
watch(configStore.isVerticalNavMini(isVerticalNavHovered), val => {
  isGroupOpen.value = val ? false : isGroupActive.value
})
</script>

<template>
  <li
    class="nav-group"
    :class="[
      {
        active: isGroupActive,
        open: isGroupOpen,
        disabled: item.disable,
      },
    ]"
  >
    <div
      class="nav-group-label"
      @click="isGroupOpen = !isGroupOpen"
    >
      <Component
        :is="layoutConfig.app.iconRenderer || 'div'"
        v-bind="item.icon || layoutConfig.verticalNav.defaultNavItemIconProps"
        class="nav-item-icon"
      />

      <!-- 👉 Title -->
      <!-- TransitionGroup 拆成個別 Transition：v-show 切換時 Vue 3 仍會套 enter/leave class，
           動畫效果保留；同時消除 TransitionGroup FLIP children 追蹤開銷 -->
      <Transition name="transition-slide-x">
        <Component
          :is=" layoutConfig.app.i18n.enable ? 'i18n-t' : 'span'"
          v-show="!hideTitleAndBadge"
          v-bind="titleI18nProps"
          class="nav-item-title"
        >
          {{ item.title }}
        </Component>
      </Transition>

      <!-- 👉 Badge -->
      <Transition
        v-if="item.badgeContent"
        name="transition-slide-x"
      >
        <Component
          :is="layoutConfig.app.i18n.enable ? 'i18n-t' : 'span'"
          v-show="!hideTitleAndBadge"
          v-bind="badgeI18nProps"
          class="nav-item-badge"
          :class="item.badgeClass"
        >
          {{ item.badgeContent }}
        </Component>
      </Transition>

      <!-- 👉 Arrow -->
      <Transition name="transition-slide-x">
        <Component
          :is="layoutConfig.app.iconRenderer || 'div'"
          v-show="!hideTitleAndBadge"
          v-bind="layoutConfig.icons.chevronRight"
          class="nav-group-arrow"
        />
      </Transition>
    </div>
    <TransitionExpand>
      <ul
        v-if="item.children"
        v-show="isGroupOpen"
        class="nav-group-children"
      >
        <Component
          :is="'children' in child ? 'VerticalNavGroup' : VerticalNavLink"
          v-for="child in processedChildren"
          :key="`title-${child.title}`"
          :item="child"
        />
      </ul>
    </TransitionExpand>
  </li>
</template>

<style lang="scss">
.layout-vertical-nav {
  .nav-group {
    &-label {
      display: flex;
      align-items: center;
      cursor: pointer;
    }
  }
}
</style>
