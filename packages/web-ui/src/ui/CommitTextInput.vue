<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import TextInput from './TextInput.vue'

defineOptions({ inheritAttrs: false })
const props = defineProps<{
  modelValue: string
  /**
   * A secret is stored that the page never receives. The field stands filled
   * with a mask in its place, empties for typing when focused, and left empty
   * keeps what is stored.
   */
  stored?: boolean
}>()
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

/** Stands in for a stored secret; it is never committed. */
const STORED_MASK = '•'.repeat(24)
const shown = computed({
  get: () => (props.stored && !focused.value && !draft.value ? STORED_MASK : draft.value),
  set: (value) => { draft.value = value },
})

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
    v-model="shown"
    @focus="focused = true"
    @blur="commit"
    @keydown.enter="finish"
    @keydown.escape="finish"
  />
</template>
