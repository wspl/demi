<script setup lang="ts">
import { onMounted, ref } from 'vue'

/**
 * A title edited where it stands. It takes the look of the text it replaces
 * from its class, arrives focused with the title selected, and ends once:
 * Enter or leaving it submits the trimmed title, Escape cancels. An empty or
 * unchanged title cancels too, so a caller only hears a real rename.
 */
const props = defineProps<{
  title: string
}>()

const emit = defineEmits<{
  submit: [title: string]
  cancel: []
}>()

const input = ref<HTMLInputElement>()
const value = ref(props.title)
// Enter ends the edit and the input then loses focus; the blur must not end it again.
let settled = false

function submit(): void {
  if (settled) {
    return
  }
  settled = true
  const title = value.value.trim()
  if (title && title !== props.title) {
    emit('submit', title)
  } else {
    emit('cancel')
  }
}

function cancel(): void {
  if (settled) {
    return
  }
  settled = true
  emit('cancel')
}

onMounted(() => {
  input.value?.focus()
  input.value?.select()
})
</script>

<template>
  <input
    ref="input"
    v-model="value"
    class="min-w-0 bg-transparent outline-none"
    aria-label="Title"
    @keydown.enter.stop="submit"
    @keydown.escape.stop="cancel"
    @keydown.stop
    @blur="submit"
    @click.stop
  />
</template>
