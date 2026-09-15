<script setup>
import { OverlayScrollbarsComponent } from 'overlayscrollbars-vue'
import {
  VList,
  VListItem,
} from 'vuetify/components/VList'

const props = defineProps({
  isDialogVisible: {
    type: Boolean,
    required: true,
  },
  searchResults: {
    type: Array,
    required: true,
  },
  isLoading: {
    type: Boolean,
    required: false,
  },
})

const emit = defineEmits([
  'update:isDialogVisible',
  'search',
])


// 👉 Hotkey

// eslint-disable-next-line camelcase
const { ctrl_k, meta_k } = useMagicKeys({
  passive: false,
  onEventFired(e) {
    if (e.ctrlKey && e.key === 'k' && e.type === 'keydown')
      e.preventDefault()
  },
})

const refSearchList = ref()
const refSearchInput = ref()
const searchQueryLocal = ref('')

// 👉 watching control + / to open dialog

/* eslint-disable camelcase */
watch([
  ctrl_k,
  meta_k,
], () => {
  emit('update:isDialogVisible', true)
})

/* eslint-enable */

// 👉 clear search result and close the dialog
const clearSearchAndCloseDialog = () => {
  searchQueryLocal.value = ''
  emit('update:isDialogVisible', false)
}

const getFocusOnSearchList = e => {
  if (e.key === 'ArrowDown') {
    e.preventDefault()
    refSearchList.value?.focus('next')
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    refSearchList.value?.focus('prev')
  }
}

const dialogModelValueUpdate = val => {
  searchQueryLocal.value = ''
  emit('update:isDialogVisible', val)
}

watch(() => props.isDialogVisible, () => {
  searchQueryLocal.value = ''
})
</script>

<template>
  <VDialog
    max-width="600"
    :model-value="props.isDialogVisible"
    :height="$vuetify.display.smAndUp ? '531' : '100%'"
    :fullscreen="$vuetify.display.width < 600"
    class="app-bar-search-dialog"
    @update:model-value="dialogModelValueUpdate"
    @keyup.esc="clearSearchAndCloseDialog"
  >
    <VCard
      height="100%"
      width="100%"
      class="position-relative"
    >
      <VCardText
        class="px-4"
        style="padding-block: 1rem 1.2rem;"
      >
        <!-- 👉 Search Input -->
        <VTextField
          ref="refSearchInput"
          v-model="searchQueryLocal"
          autofocus
          density="compact"
          variant="plain"
          class="app-bar-search-input"
          @keyup.esc="clearSearchAndCloseDialog"
          @keydown="getFocusOnSearchList"
          @update:model-value="$emit('search', searchQueryLocal)"
        >
          <!-- 👉 Prepend Inner -->
          <template #prepend-inner>
            <div class="d-flex align-center text-high-emphasis me-1">
              <VIcon
                size="24"
                icon="tabler-search"
              />
            </div>
          </template>

          <!-- 👉 Append Inner -->
          <template #append-inner>
            <div class="d-flex align-start">
              <div
                class="text-base text-disabled cursor-pointer me-3"
                @click="clearSearchAndCloseDialog"
              >
                [esc]
              </div>

              <VIcon
                icon="tabler-x"
                size="24"
                @click="clearSearchAndCloseDialog"
              />
            </div>
          </template>
        </VTextField>
      </VCardText>

      <!-- 👉 Divider -->
      <VDivider />

      <!-- 👉 Perfect Scrollbar -->
      <OverlayScrollbarsComponent
        :options="{ overflow: { x: 'hidden' }, scrollbars: { autoHide: 'leave', autoHideDelay: 200 } }"
        defer
        class="h-full"
      >
        <!-- 👉 Suggestions -->
        <div
          v-show="!!props.searchResults && !searchQueryLocal && $slots.suggestions"
          class="h-full"
        >
          <slot name="suggestions" />
        </div>

        <template v-if="!isLoading">
          <!-- 👉 Search List -->
          <VList
            v-show="searchQueryLocal.length && !!props.searchResults.length"
            ref="refSearchList"
            density="compact"
            class="app-bar-search-list py-0"
          >
            <!-- 👉 list Item /List Sub header -->
            <template
              v-for="item in props.searchResults"
              :key="item"
            >
              <slot
                name="searchResult"
                :item="item"
              >
                <VListItem>
                  {{ item }}
                </VListItem>
              </slot>
            </template>
          </VList>

          <!-- 👉 No Data found -->
          <div
            v-show="!props.searchResults.length && searchQueryLocal.length"
            class="h-full"
          >
            <slot name="noData">
              <VCardText class="h-full">
                <div class="app-bar-search-suggestions d-flex flex-column align-center justify-center text-high-emphasis pa-12">
                  <VIcon
                    size="64"
                    icon="tabler-file-alert"
                  />
                  <div class="d-flex align-center flex-wrap justify-center gap-2 text-headline-small mt-3">
                    <span>No Result For </span>
                    <span>"{{ searchQueryLocal }}"</span>
                  </div>

                  <slot name="noDataSuggestion" />
                </div>
              </VCardText>
            </slot>
          </div>
        </template>

        <!-- 👉 Loading -->
        <template v-if="isLoading">
          <VSkeletonLoader
            v-for="i in 3"
            :key="i"
            type="list-item-two-line"
          />
        </template>
      </OverlayScrollbarsComponent>
    </VCard>
  </VDialog>
</template>

<style lang="scss">
.app-bar-search-suggestions {
  .app-bar-search-suggestion {
    &:hover {
      color: rgb(var(--v-theme-primary));
    }
  }
}

.app-bar-search-dialog {
  .app-bar-search-input {
    .v-field__input {
      padding-block-start: 0.2rem;
    }
  }

  .app-bar-search-list {
    .v-list-item,
    .v-list-subheader {
      font-size: 0.75rem;
      padding-inline: 1rem;
    }

    .v-list-item {
      border-radius: 6px;
      margin-block-end: 0.125rem;
      margin-inline: 0.5rem;

      .v-list-item__append {
        .enter-icon {
          visibility: hidden;
        }
      }

      &:hover,
      &:active,
      &:focus {
        .v-list-item__append {
          .enter-icon {
            visibility: visible;
          }
        }
      }
    }

    .v-list-subheader {
      line-height: 1;
      min-block-size: auto;
      padding-block: 16px 8px;
      padding-inline-start: 1rem;
      text-transform: uppercase;
    }
  }
}

@supports selector(:focus-visible) {
  .app-bar-search-dialog {
    .v-list-item:focus-visible::after {
      content: none;
    }
  }
}
</style>

<style lang="scss" scoped>
.card-list {
  --v-card-list-gap: 16px;
}
</style>
