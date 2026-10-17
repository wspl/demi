<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import TextInput from './TextInput.vue'
import type { SentenceText } from './ui-text'

defineOptions({ inheritAttrs: false })
const props = defineProps<{
  modelValue: string
  /**
   * A secret is stored that the page never receives. The field stands filled
   * with a mask in its place, selected when focused, so typing replaces it.
   * Left as it was, it keeps what is stored; emptied and left, it commits ''
   * to remove what is stored.
   */
  stored?: boolean
  /**
   * What is wrong with a value, said as the fix ("Enter a name."), or null
   * when it can be committed. A value that cannot is kept in the field, not
   * reverted, and reported through `problem` until it is fixed or Escape
   * gives the field back its committed value.
   */
  validate?: (value: string) => SentenceText | null
  /**
   * Saves a committed value and answers why it was refused, said as the
   * fix ("Port 18910 is already exposed."), or null once saved. A refused
   * value stays in the field to correct, reported through `problem` as a
   * value `validate` refuses is; without it, `commit` alone saves.
   */
  save?: (value: string) => Promise<SentenceText | null>
}>()
const emit = defineEmits<{
  commit: [value: string]
  /** What is wrong with the value kept in the field; null once nothing is. */
  problem: [message: SentenceText | null]
}>()
const draft = ref(props.modelValue)
const focused = ref(false)
/** Whether the user changed the field since it took focus: the mask of a stored secret stands until then. */
const edited = ref(false)

watch(
  () => props.modelValue,
  (value) => {
    if (!focused.value) {
      draft.value = value
    }
  },
)

/** Stands in for a stored secret; it is never committed. */
const MASK_DOT = '•'
const STORED_MASK = MASK_DOT.repeat(24)
const shown = computed({
  get: () => (props.stored && !edited.value && !draft.value ? STORED_MASK : draft.value),
  set: (value) => {
    edited.value = true
    // What is left of the mask after an edit is no part of the value.
    draft.value = props.stored ? value.replaceAll(MASK_DOT, '') : value
  },
})

function focus(event: FocusEvent): void {
  focused.value = true
  edited.value = false
  if (props.stored && !draft.value && event.target instanceof HTMLInputElement) {
    event.target.select()
  }
}

function commit(): void {
  focused.value = false
  const value = draft.value
  const removed = props.stored === true && edited.value && value === ''
  edited.value = false
  const problem = props.validate?.(value) ?? null
  emit('problem', problem)
  if (problem !== null) {
    return
  }
  if (value === props.modelValue && !removed) {
    draft.value = props.modelValue
    return
  }
  emit('commit', value)
  if (props.save) {
    void saved(props.save, value)
  } else {
    draft.value = props.modelValue
  }
}

/** The value stays in the field while it saves, and after, when refused, until edited again. */
async function saved(save: (value: string) => Promise<SentenceText | null>, value: string): Promise<void> {
  const refusal = await save(value)
  if (focused.value || draft.value !== value) {
    return
  }
  emit('problem', refusal)
  if (refusal === null) {
    draft.value = props.modelValue
  }
}

/** Enter commits and Escape reverts; either ends the edit, and the dialog around stays open. */
function finish(event: KeyboardEvent): void {
  event.preventDefault()
  if (event.key === 'Escape') {
    draft.value = props.modelValue
    edited.value = false
  }
  const target = event.target as HTMLElement
  target.blur()
}
</script>

<template>
  <TextInput
    v-bind="$attrs"
    v-model="shown"
    @focus="focus"
    @blur="commit"
    @keydown.enter="finish"
    @keydown.escape="finish"
  />
</template>
