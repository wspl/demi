<script setup lang="ts">
import { computed } from 'vue'
import { ICON_PX } from '../../ui/icon-metrics'
import type { ToolCallBlock } from '../block-types'
import { useEditSelection, useTranscript } from '../edit-selection'
import { callPillFiles, pillSelection } from '../../files/request-changes'
import FileChangePills from './FileChangePills.vue'

/**
 * The files a shell call changed, as pills under its row, but for those its
 * request does not show, such as a file in a temporary directory. A pill
 * opens its file in the call's request, at the call's first edit of it
 * (`edit-tracking.md` § What the conversation shows).
 */
const props = defineProps<{ block: ToolCallBlock }>()
const select = useEditSelection()
const transcript = useTranscript()
const files = computed(() => callPillFiles(transcript?.requests() ?? null, props.block))

/**
 * What each pill opens, by path; none while the call belongs to no request,
 * or once a later call of it removed the file.
 */
function selection(path: string) {
  return transcript ? pillSelection(transcript.node, transcript.requests(), props.block, path) : null
}
function selectable(path: string): boolean {
  return select() !== undefined && selection(path) !== null
}

function pick(path: string, clicks: number): void {
  const picked = selection(path)
  if (picked) {
    select()?.(picked, clicks)
  }
}
</script>

<template>
  <FileChangePills
    v-if="files.length > 0"
    class="py-1"
    :style="{ paddingLeft: `${ICON_PX.in28 + 8}px` }"
    :files="files"
    :selectable="selectable"
    @select="pick"
  />
</template>
