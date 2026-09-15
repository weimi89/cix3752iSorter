<script setup>
const props = defineProps({
  title: {
    type: String,
    required: true,
  },
})

const emit = defineEmits(['cancel'])
</script>

<template>
  <div class="pa-3 d-flex align-center">
    <h5 class="text-headline-small text-inherit">
      {{ props.title }}
    </h5>
    <VSpacer />
    <slot name="beforeClose" />
    <IconBtn
      size="small"
      @click="$emit('cancel', $event)"
    >
      <VIcon
        size="24"
        icon="tabler-x"
        class="text-inherit"
      />
    </IconBtn>
  </div>
</template>

<style scoped>
.text-inherit {
  color: inherit !important;
}
</style>
