<script setup>
/**
 * AppBulkInput - 高效能批量輸入組件
 * 對齊 Materio resources/js/@core/components/app-form-elements/AppBulkInput.vue
 * 用原生 input 模擬 VCombobox 樣式，避免大量 chips 時 vuetify 效能瓶頸
 */

defineOptions({
  name: 'AppBulkInput',
  inheritAttrs: false,
})

const props = defineProps({
  modelValue: {
    type: Array,
    default: () => [],
  },
  label: {
    type: String,
    default: '',
  },
  placeholder: {
    type: String,
    default: '',
  },
  delimiters: {
    type: Array,
    default: () => [',', ';', ' ', '\n', '\r\n'],
  },
  maxVisibleChips: {
    type: Number,
    default: 100,
  },
  clearable: {
    type: Boolean,
    default: true,
  },
  /// 把「清空」按鈕放在 label 右側（外露明顯按鈕）；
  /// 預設 false → 沿用 v-field 內 hover 才顯示的 ⊗ icon
  clearableTop: {
    type: Boolean,
    default: false,
  },
})

const emit = defineEmits(['update:modelValue'])

const elementId = computed(() => {
  const attrs = useAttrs()
  const _elementIdToken = attrs.id
  const _id = useId()
  return _elementIdToken ? `app-bulk-input-${_elementIdToken}` : _id
})

const internalValue = ref(new Set(props.modelValue))
const inputValue = ref('')
const isExpanded = ref(false)
const isFocused = ref(false)
const inputRef = ref(null)

// 父層每次以新陣列替換 modelValue(參考已變即觸發 watch),故不加 deep ——
// 對可能達數千筆的 SN 陣列做深層響應式遍歷是白費成本
watch(() => props.modelValue, (newVal) => {
  internalValue.value = new Set(newVal)
})

const itemCount = computed(() => internalValue.value.size)
const itemsArray = computed(() => Array.from(internalValue.value))
const shouldVirtualize = computed(() => itemCount.value > props.maxVisibleChips)

const visibleChips = computed(() => {
  if (shouldVirtualize.value && !isExpanded.value) {
    return itemsArray.value.slice(0, props.maxVisibleChips)
  }
  return itemsArray.value
})

const hiddenCount = computed(() => {
  if (shouldVirtualize.value && !isExpanded.value) {
    return itemCount.value - props.maxVisibleChips
  }
  return 0
})

const emitUpdate = () => {
  emit('update:modelValue', Array.from(internalValue.value))
}

const parseInput = (text) => {
  const delimiterRegex = new RegExp(`[${props.delimiters.map(d => d.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('')}]+`)
  return text.split(delimiterRegex).map(s => s.trim()).filter(Boolean)
}

const addItems = (items) => {
  let hasNew = false
  for (const item of items) {
    if (item && !internalValue.value.has(item)) {
      internalValue.value.add(item)
      hasNew = true
    }
  }
  if (hasNew) emitUpdate()
}

const removeItem = (item) => {
  internalValue.value.delete(item)
  emitUpdate()
}

const clearAll = () => {
  internalValue.value.clear()
  emitUpdate()
}

const handleKeydown = (event) => {
  if (event.key === 'Enter' && inputValue.value.trim()) {
    event.preventDefault()
    const items = parseInput(inputValue.value)
    addItems(items)
    inputValue.value = ''
  }
  if (event.key === 'Backspace' && !inputValue.value && itemCount.value > 0) {
    const lastItem = itemsArray.value[itemsArray.value.length - 1]
    removeItem(lastItem)
  }
}

const handlePaste = (event) => {
  event.preventDefault()
  const pastedData = event.clipboardData?.getData('Text')
  if (!pastedData) return

  requestAnimationFrame(() => {
    const items = parseInput(pastedData)
    addItems(items)
    inputValue.value = ''
  })
}

const handleBlur = () => {
  isFocused.value = false
  if (inputValue.value.trim()) {
    const items = parseInput(inputValue.value)
    addItems(items)
    inputValue.value = ''
  }
}

const focusInput = () => {
  inputRef.value?.focus()
}

const toggleExpand = () => {
  isExpanded.value = !isExpanded.value
}

// 對外公開:讓掃描頁可在載入 / 清空後把游標移回輸入框,操作員免手動點擊
defineExpose({ focusInput })
</script>

<template>
  <div class="app-bulk-input flex-grow-1" :class="$attrs.class">
    <!-- clearableTop=true 時 label 與「清空」按鈕並排在頂部 -->
    <div v-if="label || (clearableTop && clearable && itemCount > 0)" class="label-row d-flex align-center mb-1">
      <VLabel
        v-if="label"
        :for="elementId"
        class="text-body-medium text-wrap"
        style="line-height: 15px;"
      >
        {{ label }}
        <span v-if="itemCount > 0" class="item-count ms-2">{{ $t('bulk.itemsCount', { n: itemCount }) }}</span>
      </VLabel>
      <button
        v-if="clearableTop && clearable && itemCount > 0"
        type="button"
        class="clear-all-btn"
        @click="clearAll"
      >
        <VIcon icon="tabler-circle-x" size="14" class="me-1" />{{ $t('common.clearAll') }}
      </button>
    </div>

    <div
      class="v-input v-input--horizontal v-input--density-default v-combobox v-text-field"
      :class="{ 'v-input--focused': isFocused }"
    >
      <div class="v-input__control">
        <div
          class="v-field v-field--variant-outlined"
          :class="{
            'v-field--focused': isFocused,
            'v-field--active': itemCount > 0 || isFocused
          }"
          @click="focusInput"
        >
          <div class="v-field__overlay" />
          <div class="v-field__field">
            <div
              class="v-field__input"
              :class="{ 'chips-expanded': isExpanded }"
            >
              <VChip
                v-for="item in visibleChips"
                :key="item"
                size="small"
                closable
                class="chip-item"
                @click:close="removeItem(item)"
              >
                {{ item }}
              </VChip>

              <VChip
                v-if="hiddenCount > 0"
                size="small"
                color="primary"
                variant="outlined"
                @click.stop="toggleExpand"
              >
                {{ isExpanded ? $t('bulk.collapse') : $t('bulk.expandMore', { n: hiddenCount }) }}
              </VChip>

              <input
                :id="elementId"
                ref="inputRef"
                v-model="inputValue"
                type="text"
                :placeholder="itemCount === 0 ? placeholder : ''"
                @focus="isFocused = true"
                @blur="handleBlur"
                @keydown="handleKeydown"
                @paste="handlePaste"
              >
            </div>
          </div>
          <!-- clearableTop=true 時改用上方按鈕，這裡不再顯示 -->
          <div v-if="!clearableTop && clearable && itemCount > 0" class="v-field__clearable">
            <VIcon
              icon="tabler-circle-x"
              size="16"
              @click.stop="clearAll"
            />
          </div>
          <div class="v-field__outline">
            <div class="v-field__outline__start" />
            <div class="v-field__outline__end" />
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style lang="scss" scoped>
.app-bulk-input {
  .v-input { flex: 1 1 auto; }
  .v-field { cursor: text; }

  .v-field__field {
    display: flex !important;
    align-items: center !important;
    flex: 1 1 auto;
  }

  .v-field__input {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    padding: 6px 0 6px 16px;
    min-block-size: 38px;
    max-block-size: 120px;
    overflow-y: auto;
    flex: 1 1 auto;

    &.chips-expanded { max-block-size: 300px; }

    input {
      flex: 1 1 auto;
      min-inline-size: 60px;
      block-size: 24px;
      border: none;
      outline: none;
      background: transparent;
      font-size: 1rem;
      line-height: 24px;
      color: rgba(var(--v-theme-on-surface), var(--v-high-emphasis-opacity));
      padding: 0;

      &::placeholder {
        color: rgba(var(--v-theme-on-surface), 0.38);
        opacity: 1;
      }
    }
  }

  .chip-item {
    contain: layout style;
  }

  .item-count {
    font-size: 0.75rem;
    color: rgb(var(--v-theme-primary));
  }

  .v-field__clearable {
    display: flex;
    align-items: center;
    margin-inline-start: 4px;
    margin-inline-end: 14px;
    padding: 0;
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.15s ease-in-out;
  }

  .v-field:hover .v-field__clearable,
  .v-field--focused .v-field__clearable { opacity: 1; }

  // label row 用 relative + button 用 absolute,讓「清空」按鈕完全脫離高度計算
  // → 按鈕出現/消失不會撐高 row,row 永遠等於 label 原本的 line-height
  .label-row {
    position: relative;
  }

  // clearableTop=true 時 label 旁的「清空」按鈕
  .clear-all-btn {
    position: absolute;
    inset-block-start: 50%;
    inset-inline-end: 0;
    transform: translateY(-50%);
    display: inline-flex;
    align-items: center;
    font-size: 0.75rem;
    color: rgba(var(--v-theme-on-surface), 0.6);
    padding-inline: 8px;
    padding-block: 0;
    border-radius: 4px;
    cursor: pointer;
    transition: background 0.15s, color 0.15s;
    background: transparent;
    border: none;

    &:hover {
      background: rgb(var(--v-theme-error) / 0.08);
      color: rgb(var(--v-theme-error));
    }
  }

  .v-field__outline {
    --v-field-border-opacity: 0.22;
  }

  .v-field--focused .v-field__outline {
    --v-field-border-width: 2px;
    --v-field-border-opacity: 1;
    color: rgb(var(--v-theme-primary));
  }
}
</style>
