<script setup lang="ts">
import { computed } from 'vue'
import { Check, CircleX, RotateCw, X } from '@lucide/vue'
import Button from '../ui/Button.vue'
import IconButton from '../ui/IconButton.vue'
import ProgressBar from '../ui/ProgressBar.vue'
import Tooltip from '../ui/Tooltip.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import { ICON_PX } from '../ui/icon-metrics'
import FileIcon from './FileIcon.vue'
import { uploadFiles, uploadSize, type FileUpload, type FileUploads } from './file-uploads'
import { formatBytes } from './format'
import { baseName } from './paths'

/**
 * A source's uploads, and nothing while there are none; its host sets its
 * height, and the rows scroll inside it. The one on its way comes first, then
 * those waiting, the failed ones, and the completed ones last, each group in
 * the order it was asked for. A folder is one row for everything in it.
 *
 * A row is two lines, the name and one line of facts, and the one on its way
 * adds a bar between them: how much has gone and how fast, and of a folder
 * how many of its files; a waiting one its size, a completed one a check, a
 * failed one a cross and why, or of a folder how many of its files failed. A
 * line that does not fit is cut, and its tooltip shows it whole; the name's
 * shows where the upload goes, and a folder's failures list in theirs.
 * Cancel stops an upload and drops it, Retry sends what failed again, and
 * Clear drops the finished ones, which can also be dismissed one by one.
 */
const props = defineProps<{
  uploads: FileUploads
}>()

/** Waiting or on its way: what Cancel stops, where a finished upload is dismissed instead. */
function underWay(upload: FileUpload): boolean {
  return upload.state.phase === 'waiting' || upload.state.phase === 'uploading'
}

const finished = computed(() => props.uploads.items.some((upload) => !underWay(upload)))

/** Where each phase sorts: what still needs attention above what is done. */
const PHASE_ORDER = { uploading: 0, waiting: 1, failed: 2, done: 3 } as const

const ordered = computed(() =>
  [...props.uploads.items].sort((a, b) => PHASE_ORDER[a.state.phase] - PHASE_ORDER[b.state.phase]))

function stop(upload: FileUpload): void {
  if (underWay(upload))
    props.uploads.cancel(upload.id)
  else
    props.uploads.dismiss(upload.id)
}

/** A folder's files as a count: `40 files`, or `12 / 40 files` of those landed. */
function fileCount(upload: FileUpload, landed: boolean): string {
  const files = uploadFiles(upload)
  const total = `${files.length} ${files.length === 1 ? 'file' : 'files'}`
  return landed ? `${files.filter((step) => step.done).length} / ${total}` : total
}

/** The line under the name: an upload's facts, or why it failed. */
function facts(upload: FileUpload): string {
  const folder = upload.kind === 'folder'
  const size = formatBytes(uploadSize(upload))
  const state = upload.state
  if (state.phase === 'waiting')
    return (folder ? ['Waiting', fileCount(upload, false), size] : ['Waiting', size]).join(' · ')
  if (state.phase === 'uploading') {
    const parts = folder ? [fileCount(upload, true)] : []
    parts.push(`${formatBytes(state.sent)} / ${size}`)
    if (state.rate !== null)
      parts.push(`${formatBytes(state.rate)}/s`)
    return parts.join(' · ')
  }
  if (state.phase === 'done')
    return folder ? `Completed · ${fileCount(upload, false)}` : 'Completed'
  // One failure says itself; a folder's several say how many.
  if (state.failures.length > 1)
    return `${state.failures.length} of ${fileCount(upload, false)} failed`
  const failure = state.failures[0]!
  return failure.path ? `${failure.path}: ${failure.message}` : failure.message
}

/** How many of a folder's failures its tooltip lists before it says how many more. */
const FAILURES_LISTED = 8
</script>

<template>
  <section
    v-if="uploads.items.length > 0"
    class="flex min-h-0 flex-col border-t border-line bg-surface-editor"
    aria-label="Uploads"
  >
    <div class="flex h-8 shrink-0 select-none items-center pl-3 pr-1 text-chrome font-medium text-fg-muted">
      <span class="min-w-0 flex-1 truncate">Uploads</span>
      <Button v-if="finished" size="sm" variant="ghost" class="shrink-0" @click="uploads.clearFinished()">Clear</Button>
    </div>
    <ul class="min-h-0 flex-1 overflow-y-auto px-1 pb-1">
      <!-- The name and its controls share the first line; what follows runs under the controls too. -->
      <li
        v-for="upload in ordered"
        :key="upload.id"
        class="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-1.5 rounded-md py-1 pl-2 pr-1"
      >
        <FileIcon :name="baseName(upload.path)" :is-directory="upload.kind === 'folder'" />
        <Tooltip tag="div" class="min-w-0 truncate text-chrome leading-5 text-fg-body" :content="upload.path">
          {{ baseName(upload.path) }}
        </Tooltip>
        <div class="flex shrink-0 items-center">
          <Tooltip v-if="upload.state.phase === 'failed'" content="Retry">
            <IconButton :icon="RotateCw" size="xs" variant="ghost" aria-label="Retry" spin-on-click @click="uploads.retry(upload.id)" />
          </Tooltip>
          <Tooltip :content="underWay(upload) ? 'Cancel' : 'Dismiss'">
            <IconButton
              :icon="X"
              size="xs"
              variant="ghost"
              :aria-label="underWay(upload) ? 'Cancel' : 'Dismiss'"
              @click="stop(upload)"
            />
          </Tooltip>
        </div>
        <div class="col-span-2 col-start-2 min-w-0 text-[11px] leading-4">
          <ProgressBar
            v-if="upload.state.phase === 'uploading'"
            class="my-1"
            :value="upload.state.sent"
            :max="uploadSize(upload)"
            :label="baseName(upload.path)"
          />
          <div
            class="flex min-w-0 items-center gap-1 tabular-nums"
            :class="upload.state.phase === 'failed' ? 'text-on-danger' : 'text-fg-subtle'"
          >
            <Check v-if="upload.state.phase === 'done'" :size="ICON_PX.in20" class="shrink-0 text-on-success" />
            <CircleX v-else-if="upload.state.phase === 'failed'" :size="ICON_PX.in20" class="shrink-0" />
            <!-- A folder's failures are more than its line: they list in its tooltip. -->
            <Tooltip
              v-if="upload.state.phase === 'failed' && upload.state.failures.length > 1"
              tag="span"
              class="block min-w-0 truncate"
            >
              {{ facts(upload) }}
              <template #overlay>
                <ul>
                  <li v-for="failure in upload.state.failures.slice(0, FAILURES_LISTED)" :key="failure.path" class="break-words">
                    {{ failure.path }}: {{ failure.message }}
                  </li>
                  <li v-if="upload.state.failures.length > FAILURES_LISTED">
                    and {{ upload.state.failures.length - FAILURES_LISTED }} more
                  </li>
                </ul>
              </template>
            </Tooltip>
            <TruncatedText v-else :text="facts(upload)" />
          </div>
        </div>
      </li>
    </ul>
  </section>
</template>
