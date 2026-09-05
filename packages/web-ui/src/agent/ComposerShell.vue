<script setup lang="ts">
import { computed, ref } from 'vue'
import { dataTransferFiles, transferHasFiles } from './message-input/attachments'

const props = defineProps<{
  focused?: boolean
  expanded?: boolean
  dropping?: boolean
}>()

const emit = defineEmits<{
  dropFiles: [files: File[]]
}>()

const dragDepth = ref(0)
const showDrop = computed(() => props.dropping === true || dragDepth.value > 0)

function onDragEnter(event: DragEvent): void {
  if (!transferHasFiles(event.dataTransfer)) return
  event.preventDefault()
  dragDepth.value += 1
}

function onDragOver(event: DragEvent): void {
  if (!transferHasFiles(event.dataTransfer)) return
  event.preventDefault()
  if (event.dataTransfer) event.dataTransfer.dropEffect = 'copy'
}

function onDragLeave(event: DragEvent): void {
  if (!transferHasFiles(event.dataTransfer)) return
  dragDepth.value = Math.max(0, dragDepth.value - 1)
}

function onDrop(event: DragEvent): void {
  if (!transferHasFiles(event.dataTransfer)) return
  event.preventDefault()
  dragDepth.value = 0
  const files = event.dataTransfer ? dataTransferFiles(event.dataTransfer) : []
  if (files.length > 0) emit('dropFiles', files)
}
</script>

<template>
  <div
    class="input-float composer-shell bg-surface-raised outline outline-1 transition-[outline-color] duration-200"
    :class="[
      focused || showDrop ? 'outline-line-focus' : 'outline-line',
      expanded ? 'composer-shell-expanded' : 'composer-shell-capsule',
    ]"
    :data-dropping="showDrop ? '' : undefined"
    @dragenter="onDragEnter"
    @dragover="onDragOver"
    @dragleave="onDragLeave"
    @drop="onDrop"
  >
    <div v-if="$slots.chips" class="composer-chips flex flex-wrap gap-1.5 px-3 pt-2">
      <slot name="chips" />
    </div>
    <div class="composer-attach">
      <slot name="attach" />
    </div>
    <div class="composer-editor">
      <slot name="editor" />
    </div>
    <div class="composer-model">
      <slot name="model" />
    </div>
    <div class="composer-actions flex items-center gap-1">
      <slot name="actions" />
    </div>
  </div>
</template>
