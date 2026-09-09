<script setup lang="ts">
import { computed, nextTick, ref } from 'vue'
import Button from './Button.vue'
import KeyCap from './KeyCap.vue'
import { shortcutKeyCap, shortcutModifiers } from './shortcut'

/**
 * A shortcut shown as key caps, with a Change button that records a new one in
 * place. While recording the button stays pressed and the caps show whatever is
 * held right now; a key pressed with its modifiers commits, Escape, losing focus
 * or clicking Change again cancels. Nothing else changes shape.
 */
const props = defineProps<{
  modelValue: string
  /** Button height family; the caps have their own. */
  size?: 'sm' | 'md'
}>()

const emit = defineEmits<{
  'update:modelValue': [keys: string]
}>()

const recording = ref(false)
const held = ref('')
const field = ref<HTMLElement>()

function start() {
  recording.value = true
  held.value = ''
  void nextTick(() => field.value?.focus())
}

function stop() {
  recording.value = false
  held.value = ''
}

function onKeydown(event: KeyboardEvent) {
  event.preventDefault()
  event.stopPropagation()
  if (event.key === 'Escape' && !shortcutModifiers(event)) {
    stop()
    return
  }
  const cap = shortcutKeyCap(event)
  held.value = shortcutModifiers(event) + cap
  if (cap) {
    emit('update:modelValue', held.value)
    stop()
  }
}

function onKeyup(event: KeyboardEvent) {
  if (!recording.value)
    return
  held.value = shortcutModifiers(event)
}

const shown = computed(() => (recording.value ? held.value : props.modelValue))
</script>

<template>
  <span class="inline-flex items-center gap-2">
    <span
      ref="field"
      tabindex="-1"
      class="inline-flex h-6 items-center justify-end outline-none"
      :aria-label="recording ? 'Recording shortcut' : undefined"
      @keydown="recording && onKeydown($event)"
      @keyup="onKeyup"
      @blur="stop"
    >
      <KeyCap v-if="shown" :keys="shown" />
      <span v-else class="select-none text-[11px] text-fg-subtle">Press keys…</span>
    </span>
    <!-- mousedown would blur the field and stop recording before the click lands. -->
    <span @mousedown.prevent><Button
        :size="size"
        :pressed="recording"
        @click="recording ? stop() : start()"
      >Change</Button></span>
  </span>
</template>
