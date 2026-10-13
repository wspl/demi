<script setup lang="ts">
import { computed, ref } from 'vue'
import { useElementSize } from '@vueuse/core'
import { useFileDrop } from '../composables/useFileDrop'
import DropOutline from '../ui/DropOutline.vue'
import { dataTransferFiles } from './message-input/attachments'
import { transferCopiedMessage } from './message-input/copied-message'

/**
 * The composer's frame. Files dragged over it can be dropped anywhere on it:
 * a dashed line runs inside its edge, so the target is the whole composer
 * and not one control, and the editor marks where in the text they will land.
 *
 * The box holds the attach control, the message and the actions; the `below`
 * slot is a row under it, on the right, for what belongs to the conversation
 * and not to the message (the model, the context), aligned to the box's right
 * edge. Its `below-start` slot is the same row's left end, for what lists
 * the conversation's drafts; the `list` slot is a panel under that row.
 *
 * The message's line comes first. The editor slot hears what the line offers
 * its text (`line`), so a message that outgrows it opens the shell instead of
 * running off its end.
 *
 * A ruler measures the line in both shapes: on one line it lies in the
 * editor's own cell, and in the open shell it takes the editor's column in
 * the row of controls below, which is the same width. So the editor hears one
 * width that does not change with the shape, and the controls neither jump
 * nor drive each other when the shell opens and closes.
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

const ruler = ref<HTMLElement | null>(null)
const { width: line } = useElementSize(ruler)

const shell = ref<HTMLElement | null>(null)
const { over: dragOver } = useFileDrop(shell, {
  drop(event) {
    const transfer = event.dataTransfer
    // A message dragged out of another one brings its files without their
    // bytes: the editor takes it (`intake.ts`), not its files.
    if (!transfer || transferCopiedMessage(transfer))
      return
    const files = dataTransferFiles(transfer)
    if (files.length > 0)
      emit('dropFiles', files, event)
  },
})
const showDrop = computed(() => props.dropping === true || dragOver.value)
</script>

<template>
  <div class="composer-frame">
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
    <div class="composer-below">
      <div v-if="$slots['below-start']" class="composer-below-start">
        <slot name="below-start" />
      </div>
      <slot name="below" />
    </div>
    <div v-if="$slots['list']" class="composer-list">
      <slot name="list" />
    </div>
  </div>
</template>
