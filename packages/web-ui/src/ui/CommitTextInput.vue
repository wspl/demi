<script setup lang="ts">
import { ref, watch } from 'vue'
import TextInput from './TextInput.vue'

defineOptions({ inheritAttrs: false })
const props = defineProps<{ modelValue: string }>()
const emit = defineEmits<{ commit: [value: string] }>()
const draft = ref(props.modelValue)
const focused = ref(false)

watch(
  () => props.modelValue,
  (value) => {
    if (!focused.value) {
      draft.value = value
    }
  },
)

function commit(): void {
  focused.value = false
  const value = draft.value
  draft.value = props.modelValue
  if (value !== props.modelValue) {
    emit('commit', value)
  }
}

function finish(event: KeyboardEvent): void {
  event.preventDefault()
  if (event.key === 'Escape') {
    draft.value = props.modelValue
  }
  const target = event.target as HTMLElement
  target.blur()
}
</script>

<template>
  <TextInput
    v-bind="$attrs"
    v-model="draft"
    @focus="focused = true"
    @blur="commit"
    @keydown.enter="finish"
    @keydown.escape="finish"
  />
</template>
