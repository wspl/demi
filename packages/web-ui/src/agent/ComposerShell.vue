<script setup lang="ts">
import { computed, ref } from 'vue'
import { useElementSize } from '@vueuse/core'
import { useFileDrop } from '../composables/useFileDrop'
import DropOutline from '../ui/DropOutline.vue'
import { dataTransferFiles } from './message-input/attachments'

/**
 * The composer's frame. Files dragged over it can be dropped anywhere on it:
 * a dashed line runs inside its edge, so the target is the whole composer
 * and not one control, and the editor marks where in the text they will land.
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
  /** Files dropped, with the drop that places them. */
  dropFiles: [files: File[], event: DragEvent]
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
      emit('dropFiles', files, event)
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
    <DropOutline
      v-if="showDrop"
      class="z-20"
      radius="var(--composer-shell-radius)"
    />
  </div>
</template>
