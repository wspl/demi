<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import TextInput from './TextInput.vue'
import type { SentenceText } from './ui-text'

defineOptions({ inheritAttrs: false })
const props = defineProps<{
  modelValue: string
  /**
   * A secret is stored that the page never receives. The field stands filled
   * with a mask in its place, empties for typing when focused, and left empty
   * keeps what is stored.
   */
  stored?: boolean
  /**
   * What is wrong with a value, said as the fix ("Enter a name."), or null
   * when it can be committed. A value that cannot is kept in the field, not
   * reverted, and reported through `problem` until it is fixed or Escape
   * gives the field back its committed value.
   */
  validate?: (value: string) => SentenceText | null
}>()
const emit = defineEmits<{
  commit: [value: string]
  /** What is wrong with the value kept in the field; null once nothing is. */
  problem: [message: SentenceText | null]
}>()
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
  const problem = props.validate?.(value) ?? null
  emit('problem', problem)
  if (problem !== null) {
    return
  }
  draft.value = props.modelValue
  if (value !== props.modelValue) {
    emit('commit', value)
  }
}

/** Enter commits and Escape reverts; either ends the edit, and the dialog around stays open. */
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
