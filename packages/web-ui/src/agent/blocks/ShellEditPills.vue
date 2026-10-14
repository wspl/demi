<script setup lang="ts">
import { computed } from 'vue'
import { ICON_PX } from '../../ui/icon-metrics'
import type { ToolCallBlock } from '../block-types'
import { storedShellView } from '../block-helpers'
import { useEditSelection, useTranscript } from '../edit-selection'
import { callPills, pillSelection, requestFileSelection } from '../../files/request-changes'
import FileChangePills from './FileChangePills.vue'

/**
 * The files a shell call changed, as pills under its row, as the call's
 * view stores them: what it wrote, removed (struck through) and renamed,
 * but for a file outside its agent's working directory. A pill opens its file in the
 * call's request, at the call's first edit of it (`edit-tracking.md`
 * § What the conversation shows).
 */
const props = defineProps<{ block: ToolCallBlock }>()
const select = useEditSelection()
const transcript = useTranscript()
const files = computed(() => callPills(props.block))

/**
 * What each pill opens, by path; none while the call belongs to no request,
 * for a file it only removed or renamed, or once a later call removed it.
 */
function selection(path: string) {
  const call = storedShellView(props.block)?.commandId
  return transcript && call
    ? pillSelection(transcript.node, transcript.requests(), props.block, call, path, transcript.directory(props.block.id))
    : null
}
/** What the "+N Files" pill opens: the first of its files the request lists. */
function restSelection(paths: readonly string[]) {
  const call = storedShellView(props.block)?.commandId
  if (!transcript || !call)
    return null
  const directory = transcript.directory(props.block.id)
  for (const path of paths) {
    const found = pillSelection(transcript.node, transcript.requests(), props.block, call, path, directory)
      ?? requestFileSelection(transcript.node, transcript.requests(), props.block, path, directory)
    if (found)
      return found
  }
  return null
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
function pickRest(paths: readonly string[]): void {
  const picked = restSelection(paths)
  if (picked) {
    select()?.(picked)
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
    :rest-selectable="(paths) => select() !== undefined && restSelection(paths) !== null"
    @select="pick"
    @open-rest="pickRest"
  />
</template>
