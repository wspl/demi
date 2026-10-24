<script setup lang="ts">
import { computed, nextTick, ref } from 'vue'
import Button from './Button.vue'
import KeyCap from './KeyCap.vue'
import { t as shared } from '../locales/text'
import { t } from './locales/text'
import { formatShortcut, shortcutModifiers } from './shortcut'

/**
 * A shortcut shown as key caps, with a Change button that records a new one in place.
 *
 * - While recording, the button stays pressed and the caps show whatever is held
 *   right now.
 * - A key pressed with its modifiers commits, written in the page's set (`shortcut.ts`);
 *   on a PC, a key pressed with the Windows key commits nothing. Escape, losing focus
 *   or clicking Change again cancels.
 * - Nothing else changes shape.
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
  const keys = formatShortcut(event)
  held.value = keys || shortcutModifiers(event)
  if (keys) {
    emit('update:modelValue', keys)
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
      :aria-label="recording ? t('shortcutRecorder.recording') : undefined"
      @keydown="recording && onKeydown($event)"
      @keyup="onKeyup"
      @blur="stop"
    >
      <KeyCap v-if="shown" :keys="shown" />
      <span v-else class="select-none text-[11px] text-fg-subtle">{{ t('shortcutRecorder.prompt') }}</span>
    </span>
    <!-- mousedown would blur the field and stop recording before the click lands. -->
    <span @mousedown.prevent><Button
        :size="size"
        :pressed="recording"
        @click="recording ? stop() : start()"
      >{{ shared('common.change') }}</Button></span>
  </span>
</template>
