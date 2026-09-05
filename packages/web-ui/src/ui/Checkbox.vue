<script setup lang="ts">
import { computed } from 'vue'
import { checkboxMark, nextCheckbox } from './checkbox'

defineProps<{
  label: string
}>()

const checked = defineModel<boolean>({ required: true })
const partial = defineModel<boolean>('partial', { default: false })

const mark = computed(() => checkboxMark(checked.value, partial.value))

function toggle() {
  const next = nextCheckbox(checked.value, partial.value)
  checked.value = next.checked
  partial.value = next.partial
}

function onKeydown(event: KeyboardEvent) {
  if (event.key !== ' ' && event.key !== 'Enter') return
  event.preventDefault()
  toggle()
}
</script>

<template>
  <span
    class="group inline-flex h-7 cursor-default items-center gap-2"
    role="checkbox"
    tabindex="0"
    :aria-checked="partial ? 'mixed' : checked"
    @click="toggle"
    @keydown="onKeydown"
  >
    <span class="checkbox-mark" :data-state="mark" />
    <span class="select-none text-chrome text-fg-body">{{ label }}</span>
  </span>
</template>
