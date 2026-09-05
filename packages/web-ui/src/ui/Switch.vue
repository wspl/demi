<script setup lang="ts">
const props = withDefaults(defineProps<{
  modelValue: boolean
  label?: string
  size?: 'sm' | 'md'
}>(), {
  size: 'md',
})

const emit = defineEmits<{
  'update:modelValue': [value: boolean]
}>()

function toggle() {
  emit('update:modelValue', !props.modelValue)
}

function onKeydown(event: KeyboardEvent) {
  if (event.key !== ' ' && event.key !== 'Enter') return
  event.preventDefault()
  toggle()
}
</script>

<template>
  <span
    class="inline-flex cursor-default items-center gap-1.5"
    role="switch"
    tabindex="0"
    :aria-checked="modelValue"
    @click="toggle"
    @keydown="onKeydown"
  >
    <span v-if="label" class="select-none text-[12px] text-fg-subtle">{{ label }}</span>
    <span
      class="inline-flex shrink-0 items-center rounded-full transition-colors duration-200 ease-out"
      :class="[
        modelValue ? 'switch-on' : 'bg-overlay/10',
        size === 'sm' ? 'h-4 w-7 p-0.5' : 'h-5 w-9 p-0.5',
      ]"
    >
      <span
        class="rounded-full bg-white shadow-sm transition-transform duration-200 ease-out"
        :class="[
          size === 'sm' ? 'size-3' : 'size-4',
          modelValue ? 'translate-x-full' : 'translate-x-0',
        ]"
      />
    </span>
  </span>
</template>

<style scoped>
.switch-on {
  background: var(--on-accent);
}
</style>
