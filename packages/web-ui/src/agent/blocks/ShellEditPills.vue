<script setup lang="ts">
import { computed } from 'vue'
import { ICON_PX } from '../../ui/icon-metrics'
import type { ToolCallBlock } from '../block-types'
import { storedShellView } from '../block-helpers'
import { useEditSelection, useTranscript } from '../edit-selection'
import { pillSelection } from '../../files/request-changes'
import FileChangePills from './FileChangePills.vue'

/**
 * The files a shell call changed, as pills under its row. A pill opens its
 * file in the call's request, at the call's first edit of it
 * (`edit-tracking.md` § What the conversation shows).
 */
const props = defineProps<{ block: ToolCallBlock }>()
const call = computed(() => storedShellView(props.block))
const select = useEditSelection()
const transcript = useTranscript()

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

function pick(path: string): void {
  const picked = selection(path)
  if (picked) {
    select()?.(picked)
  }
}
</script>

<template>
  <FileChangePills
    v-if="call?.files && call.files.length > 0"
    class="py-1"
    :style="{ paddingLeft: `${ICON_PX.in28 + 8}px` }"
    :files="call.files"
    :selectable="selectable"
    @select="pick"
  />
</template>
