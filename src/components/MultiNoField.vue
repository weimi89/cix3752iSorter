<script setup>
// 多組單號輸入框:貼上或輸入的單號立刻拆成一顆顆標籤(逗號 / 分號 / 頓號 / 空白 / Tab / 換行都算分隔,
// 貼 Excel 直欄也行),看得出拆成了幾筆、也能逐筆刪。對外仍是一個字串(空白分隔),
// 後端 split_nos 拆開後做精確比對(不做模糊)。標籤有增減就自動查,不必再按 Enter。
const model = defineModel({ type: String, default: '' })
const emit = defineEmits(['search'])

const DELIMITERS = [',', ';', '、', ' ', '\t', '\n', '\r\n']

const toList = s => (s || '').split(/[,;、\s]+/).map(v => v.trim()).filter(Boolean)

const list = ref(toList(model.value))

let timer = null
const searchSoon = () => {
  clearTimeout(timer)
  timer = setTimeout(() => emit('search'), 250)
}

// 外部清空(重設按鈕)或改值時同步進來,並照樣自動查——重設後列表要跟著回到全部
watch(model, v => {
  const next = toList(v)
  if (next.join(' ') === list.value.join(' ')) return
  list.value = next
  searchSoon()
})

const onUpdate = v => {
  list.value = v
  model.value = v.join(' ')
  searchSoon()
}
</script>

<template>
  <AppBulkInput
    :model-value="list"
    :placeholder="$t('common.multiNoPlaceholder')"
    :delimiters="DELIMITERS"
    clearable
    @update:model-value="onUpdate"
  />
</template>
