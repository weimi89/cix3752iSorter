<script setup>
import { VNumberInput } from 'vuetify/components/VNumberInput'

defineOptions({
  name: 'AppNumberInput',
  inheritAttrs: false,
})

const elementId = computed(() => {
  const attrs = useAttrs()
  const _elementIdToken = attrs.id
  const _id = useId()

  return _elementIdToken ? `app-number-input-${ _elementIdToken } }` : _id
})

const label = computed(() => useAttrs().label)
</script>

<template>
  <div
    class="app-text-field flex-grow-1"
    :class="$attrs.class"
  >
    <VLabel
      v-if="label"
      :for="elementId"
      class="mb-1 text-body-medium text-wrap"
      style="line-height: 15px;"
      :text="label"
    />

    <VNumberInput
      v-bind="{
        hideDetails: 'auto',
        ...$attrs,
        class: null,
        label: undefined,
        variant: 'outlined',
        id: elementId,
        density: 'comfortable',
        controlVariant: 'stacked',
      }"
    >
      <template
        v-for="(_, name) in $slots"
        #[name]="slotProps"
      >
        <slot
          :name="name"
          v-bind="slotProps || {}"
        />
      </template>
    </VNumberInput>
  </div>
</template>
