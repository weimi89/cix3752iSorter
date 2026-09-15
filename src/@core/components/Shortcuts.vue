<script setup>
import { OverlayScrollbarsComponent } from 'overlayscrollbars-vue'

const props = defineProps({
  togglerIcon: {
    type: String,
    required: false,
    default: 'tabler-layout-grid-add',
  },
  shortcuts: {
    type: Array,
    required: true,
  },
  secretShortcuts: {
    type: Array,
    required: false,
    default: () => [],
  },
})

const currentTab = ref('shortcuts')

// 是否顯示分頁（只有在有解鎖功能時才顯示）
const showTabs = computed(() => props.secretShortcuts.length > 0)

// 當前顯示的快捷方式
const displayedShortcuts = computed(() => {
  if (currentTab.value === 'secrets') {
    return props.secretShortcuts
  }
  return props.shortcuts
})
</script>

<template>
  <IconBtn>
    <VIcon :icon="props.togglerIcon" />

    <VMenu
      activator="parent"
      offset="12px"
      location="bottom end"
    >
      <VCard
        :width="$vuetify.display.smAndDown ? 330 : 380"
        max-height="560"
        class="d-flex flex-column"
      >
        <VCardItem class="py-2 px-3 h-12">
          <!-- 有分頁時顯示 Tabs -->
          <div v-if="showTabs" class="d-flex justify-space-between">
            <VBtn
              :variant="currentTab === 'shortcuts' ? 'flat' : 'text'"
              :color="currentTab === 'shortcuts' ? 'primary' : 'default'"
              size="small"
              @click.stop="currentTab = 'shortcuts'"
            >
              <VIcon icon="tabler-rocket" size="18" class="me-1" />
              功能快捷
            </VBtn>
            <VBtn
              :variant="currentTab === 'secrets' ? 'flat' : 'text'"
              :color="currentTab === 'secrets' ? 'warning' : 'default'"
              size="small"
              @click.stop="currentTab = 'secrets'"
            >
              <VIcon icon="tabler-lock-open" size="18" class="me-1" />
              功能解鎖
            </VBtn>
          </div>

          <!-- 沒有分頁時顯示標題 -->
          <h6 v-else class="text-base font-weight-medium">
            <VIcon icon="tabler-rocket" size="18" class="me-1" />
            功能快捷
          </h6>
        </VCardItem>

        <VDivider />

        <OverlayScrollbarsComponent :options="{ overflow: { x: 'hidden' }, scrollbars: { autoHide: 'leave', autoHideDelay: 200 } }" defer>
          <!-- 無項目時的提示 -->
          <div
            v-if="displayedShortcuts.length === 0"
            class="text-center pa-6 text-medium-emphasis"
          >
            <VIcon icon="tabler-inbox" size="48" class="mb-2" />
            <p class="mb-0">暫無項目</p>
          </div>

          <!-- 快捷方式列表 -->
          <VRow v-else class="ma-0 mt-n1">
            <VCol
              v-for="(shortcut, index) in displayedShortcuts"
              :key="shortcut.title"
              cols="6"
              class="text-center border-t cursor-pointer pa-6 shortcut-icon"
              :class="(index + 1) % 2 ? 'border-e' : ''"
              @click="shortcut?.action ? shortcut.action() : (shortcut?.to ? router.visit(route(shortcut.to.name, shortcut.to.params)) : null)"
            >
              <VAvatar
                variant="tonal"
                size="50"
                :color="currentTab === 'secrets' ? 'warning' : undefined"
              >
                <VIcon
                  size="30"
                  :color="currentTab === 'secrets' ? 'warning' : 'high-emphasis'"
                  :icon="shortcut.icon"
                />
              </VAvatar>

              <h6 class="text-base font-weight-medium mt-3 mb-0">
                {{ shortcut.title }}
              </h6>
              <p class="text-sm mb-0">
                {{ shortcut.subtitle }}
              </p>
            </VCol>
          </VRow>
        </OverlayScrollbarsComponent>
      </VCard>
    </VMenu>
  </IconBtn>
</template>

<style lang="scss">
.shortcut-icon:hover {
  background-color: rgba(var(--v-theme-on-surface), var(--v-hover-opacity));
}
</style>
