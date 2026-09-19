<script setup lang="ts">
import { computed, ref } from 'vue'
import { Paperclip } from '@lucide/vue'
import { useElementSize } from '@vueuse/core'
import { useFileDrop } from '../composables/useFileDrop'
import DropOutline from '../ui/DropOutline.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { dataTransferFiles } from './message-input/attachments'

/**
 * The composer's frame. Files dragged over it can be dropped anywhere on it:
 * the frame lights its focus line and a dashed overlay names the action, so
 * the target is the whole composer and not one control.
 *
 * The editor comes first: the model slot hears how much wider than
 * `EDITOR_MIN_PX` the editor is (`room`, negative when narrower), so the
 * control there can drop its label to its icon before the editor narrows past
 * that.
 */

const props = defineProps<{
  focused?: boolean
  expanded?: boolean
  dropping?: boolean
}>()

const emit = defineEmits<{
  dropFiles: [files: File[]]
}>()

/** The least the editor keeps: the placeholder whole, and room to type after it. */
const EDITOR_MIN_PX = 128

const editor = ref<HTMLElement | null>(null)
const { width: editorWidth } = useElementSize(editor)
const room = computed(() => editorWidth.value - EDITOR_MIN_PX)

const shell = ref<HTMLElement | null>(null)
const { over: dragOver } = useFileDrop(shell, {
  drop(event) {
    const files = event.dataTransfer ? dataTransferFiles(event.dataTransfer) : []
    if (files.length > 0)
      emit('dropFiles', files)
  },
})
const showDrop = computed(() => props.dropping === true || dragOver.value)
</script>

<template>
  <div
    ref="shell"
    class="input-float composer-shell relative bg-surface-raised outline outline-1 transition-[outline-color] duration-200"
    :class="[
      showDrop ? 'outline-transparent' : focused ? 'outline-line-focus' : 'outline-line',
      expanded ? 'composer-shell-expanded' : 'composer-shell-capsule',
    ]"
    :data-dropping="showDrop ? '' : undefined"
  >
    <div
      v-if="$slots.chips"
      class="composer-chips flex flex-wrap gap-1.5 px-3 pt-2"
    >
      <slot name="chips" />
    </div>
    <div class="composer-attach">
      <slot name="attach" />
    </div>
    <div ref="editor" class="composer-editor">
      <slot name="editor" />
    </div>
    <div class="composer-model">
      <slot name="model" :room="room" />
    </div>
    <div class="composer-actions flex items-center gap-1">
      <slot name="actions" />
    </div>
    <template v-if="showDrop">
      <div
        class="pointer-events-none absolute inset-0 z-10 flex items-center justify-center gap-2 rounded-[var(--composer-shell-radius)] bg-surface-raised/90 text-chrome text-fg-body"
        aria-hidden="true"
      >
        <Paperclip :size="ICON_PX.in28" />
        Drop to attach
      </div>
      <DropOutline class="z-20" radius="var(--composer-shell-radius)" />
    </template>
  </div>
</template>
