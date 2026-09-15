<script setup>
/**
 * AppBulkInput - 高效能批量輸入組件
 * 專門處理大量單號輸入，避免 VCombobox 在大量 chips 時的效能問題
 * 外觀與 VCombobox 一致
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
})

const emit = defineEmits(['update:modelValue'])

const elementId = computed(() => {
  const attrs = useAttrs()
  const _elementIdToken = attrs.id
  const _id = useId()
  return _elementIdToken ? `app-bulk-input-${_elementIdToken}` : _id
})

// 內部值管理
const internalValue = ref(new Set(props.modelValue))
const inputValue = ref('')
const isExpanded = ref(false)
const isFocused = ref(false)
const inputRef = ref(null)

// 同步外部值
watch(() => props.modelValue, (newVal) => {
  internalValue.value = new Set(newVal)
}, { deep: true })

// 計算屬性
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

// 發送更新
const emitUpdate = () => {
  emit('update:modelValue', Array.from(internalValue.value))
}

// 解析輸入文字
const parseInput = (text) => {
  const delimiterRegex = new RegExp(`[${props.delimiters.map(d => d.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('')}]+`)
  return text.split(delimiterRegex).map(s => s.trim()).filter(Boolean)
}

// 添加項目
const addItems = (items) => {
  let hasNew = false
  for (const item of items) {
    if (item && !internalValue.value.has(item)) {
      internalValue.value.add(item)
      hasNew = true
    }
  }
  if (hasNew) {
    emitUpdate()
  }
}

// 移除單個項目
const removeItem = (item) => {
  internalValue.value.delete(item)
  emitUpdate()
}

// 清除所有
const clearAll = () => {
  internalValue.value.clear()
  emitUpdate()
}

// 處理鍵盤輸入
const handleKeydown = (event) => {
  if (event.key === 'Enter' && inputValue.value.trim()) {
    event.preventDefault()
    const items = parseInput(inputValue.value)
    addItems(items)
    inputValue.value = ''
  }
  // Backspace 刪除最後一個
  if (event.key === 'Backspace' && !inputValue.value && itemCount.value > 0) {
    const lastItem = itemsArray.value[itemsArray.value.length - 1]
    removeItem(lastItem)
  }
}

// 處理貼上
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

// 處理輸入框失焦
const handleBlur = () => {
  isFocused.value = false
  if (inputValue.value.trim()) {
    const items = parseInput(inputValue.value)
    addItems(items)
    inputValue.value = ''
  }
}

// 點擊容器時聚焦輸入框
const focusInput = () => {
  inputRef.value?.focus()
}

// 切換展開狀態
const toggleExpand = () => {
  isExpanded.value = !isExpanded.value
}

// 處理 VCombobox 的更新（新增項目）
const handleComboboxUpdate = (newValue) => {
  if (!Array.isArray(newValue)) return

  // 找出新增的項目
  for (const item of newValue) {
    if (item && !internalValue.value.has(item)) {
      internalValue.value.add(item)
    }
  }

  // 找出被刪除的項目（從 visibleChips）
  const newSet = new Set(newValue)
  for (const item of visibleChips.value) {
    if (!newSet.has(item)) {
      internalValue.value.delete(item)
    }
  }

  emitUpdate()
}
</script>

<template>
  <div class="app-bulk-input flex-grow-1" :class="$attrs.class">
    <VLabel
      v-if="label"
      :for="elementId"
      class="mb-1 text-body-medium text-wrap"
      style="line-height: 15px;"
    >
      {{ label }}
      <span v-if="itemCount > 0" class="item-count ms-2">
        {{ itemCount }} 筆
      </span>
    </VLabel>

    <!-- 模擬 v-input v-combobox 結構 -->
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
              <!-- Chips -->
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

              <!-- +N 筆 -->
              <VChip
                v-if="hiddenCount > 0"
                size="small"
                color="primary"
                variant="outlined"
                @click.stop="toggleExpand"
              >
                {{ isExpanded ? '收合' : `+${hiddenCount} 筆` }}
              </VChip>

              <!-- 輸入框 -->
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
          <!-- 清除按鈕 - 放在 v-field 層級 -->
          <div v-if="clearable && itemCount > 0" class="v-field__clearable">
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
  // 覆寫 v-input 預設樣式
  .v-input {
    flex: 1 1 auto;
  }

  .v-field {
    cursor: text;
  }

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

    &.chips-expanded {
      max-block-size: 300px;
    }

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
  .v-field--focused .v-field__clearable {
    opacity: 1;
  }

  // 修正 outline 樣式
  .v-field__outline {
    --v-field-border-opacity: 0.22;
  }

  // Focus 時邊框樣式
  .v-field--focused .v-field__outline {
    --v-field-border-width: 2px;
    --v-field-border-opacity: 1;
    color: rgb(var(--v-theme-primary));
  }
}
</style>
