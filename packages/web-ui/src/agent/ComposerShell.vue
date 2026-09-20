<script setup lang="ts">
import { computed, ref } from 'vue'
import { useElementSize } from '@vueuse/core'
import { useFileDrop } from '../composables/useFileDrop'
import DropOutline from '../ui/DropOutline.vue'
import { provideLabelRoom } from '../ui/label-room'
import { dataTransferFiles } from './message-input/attachments'

/**
 * The composer's frame. Files dragged over it can be dropped anywhere on it:
 * a dashed line runs inside its edge, so the target is the whole composer
 * and not one control, and the editor marks where in the text they will land.
 *
 * The message's line comes first. The editor slot hears what the line offers
 * its text (`line`), so a message that outgrows it opens the shell instead of
 * running off its end; the model slot hears how much wider than
 * `EDITOR_MIN_PX` that is (`room`, negative when narrower), so the control
 * there can drop its label to its icon before the line narrows past that.
 *
 * A ruler measures the line in both shapes: on one line it lies in the
 * editor's own cell, and in the open shell it takes the editor's column in
 * the row of controls below, which is the same width. So the two slots hear
 * one width that does not change with the shape, and the controls neither
 * jump nor drive each other when the shell opens and closes.
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

const ruler = ref<HTMLElement | null>(null)
const { width: line } = useElementSize(ruler)
provideLabelRoom(() => line.value - EDITOR_MIN_PX)

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
    <div class="composer-editor">
      <slot name="editor" :line="line" />
    </div>
    <div class="composer-model">
      <slot name="model" />
    </div>
    <div
      ref="ruler"
      class="composer-line-ruler"
      aria-hidden="true"
    />
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
