<script setup lang="ts">
import { computed, reactive } from 'vue'
import { Check, CircleX, RotateCw, X } from '@lucide/vue'
import Button from '../ui/Button.vue'
import DotLine from '../ui/DotLine.vue'
import Fold from '../ui/Fold.vue'
import FoldChevron from '../ui/FoldChevron.vue'
import IconButton from '../ui/IconButton.vue'
import ProgressBar from '../ui/ProgressBar.vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import FileIcon from './FileIcon.vue'
import { uploadFiles, uploadSize, type FileUpload, type FileUploads } from './file-uploads'
import { formatBytes } from './format'
import { baseName } from './paths'

/**
 * A source's uploads under its file tree, and nothing while there are none:
 * the one on its way first, then those waiting, the failed ones, and the
 * completed ones last, each group in the order it was asked for. A folder is
 * one row for everything in it. The one on its way shows a bar, how much has
 * gone and how fast, and of a folder how many of its files; a waiting one its
 * size, a completed one a check, a failed one a cross and why: a folder whose
 * files failed says how many, and opens to list them. A row's hover names
 * where it goes. Cancel stops an upload and drops it, Retry sends what failed
 * again, and Clear drops the finished ones, which can also be dismissed one
 * by one.
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

/** What the line under the name says of an upload waiting, on its way or completed. */
function facts(upload: FileUpload): string[] {
  const folder = upload.kind === 'folder'
  const size = formatBytes(uploadSize(upload))
  const state = upload.state
  if (state.phase === 'waiting')
    return folder ? ['Waiting', fileCount(upload, false), size] : ['Waiting', size]
  if (state.phase === 'uploading') {
    const parts = folder ? [fileCount(upload, true)] : []
    parts.push(`${formatBytes(state.sent)} / ${size}`)
    if (state.rate !== null)
      parts.push(`${formatBytes(state.rate)}/s`)
    return parts
  }
  return folder ? ['Completed', fileCount(upload, false)] : ['Completed']
}

// Folders whose failed files are listed.
const opened = reactive(new Set<string>())

function toggleFailures(id: string): void {
  if (!opened.delete(id))
    opened.add(id)
}
</script>

<template>
  <section
    v-if="uploads.items.length > 0"
    class="flex max-h-60 shrink-0 flex-col border-t border-line bg-surface-editor"
    aria-label="Uploads"
  >
    <div class="flex h-8 shrink-0 select-none items-center pl-3 pr-1 text-chrome font-medium text-fg-muted">
      <span class="min-w-0 flex-1 truncate">Uploads</span>
      <Button v-if="finished" size="sm" variant="ghost" class="shrink-0" @click="uploads.clearFinished()">Clear</Button>
    </div>
    <ul class="min-h-0 overflow-y-auto px-1 pb-1">
      <!-- The name and its controls share the first line; what follows runs under the controls too. -->
      <li
        v-for="upload in ordered"
        :key="upload.id"
        class="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-1.5 rounded-md py-1 pl-2 pr-1"
      >
        <FileIcon :name="baseName(upload.path)" :is-directory="upload.kind === 'folder'" />
        <div class="truncate text-chrome leading-5 text-fg-body" :title="upload.path">{{ baseName(upload.path) }}</div>
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
          <template v-if="upload.state.phase === 'failed'">
            <!-- One failure says itself; a folder's several say how many, and open to list them. -->
            <div v-if="upload.state.failures.length === 1" class="flex min-w-0 items-start gap-1 text-on-danger">
              <CircleX :size="ICON_PX.in20" class="mt-0.5 shrink-0" />
              <span class="line-clamp-2">
                <template v-if="upload.state.failures[0]!.path">{{ upload.state.failures[0]!.path }}: </template>{{ upload.state.failures[0]!.message }}
              </span>
            </div>
            <template v-else>
              <button
                type="button"
                class="flex max-w-full cursor-default items-center gap-1 text-on-danger"
                :aria-expanded="opened.has(upload.id)"
                @click="toggleFailures(upload.id)"
              >
                <CircleX :size="ICON_PX.in20" class="shrink-0" />
                <span class="truncate">{{ upload.state.failures.length }} of {{ fileCount(upload, false) }} failed</span>
                <FoldChevron :open="opened.has(upload.id)" :size="ICON_PX.in12" />
              </button>
              <Fold :open="opened.has(upload.id)">
                <ul class="pl-4 pt-0.5">
                  <li
                    v-for="failure in upload.state.failures"
                    :key="failure.path"
                    class="truncate text-fg-subtle"
                    :title="`${failure.path}: ${failure.message}`"
                  >
                    <span class="text-fg-muted">{{ failure.path }}</span> · {{ failure.message }}
                  </li>
                </ul>
              </Fold>
            </template>
          </template>
          <template v-else>
            <ProgressBar
              v-if="upload.state.phase === 'uploading'"
              class="my-1"
              :value="upload.state.sent"
              :max="uploadSize(upload)"
              :label="baseName(upload.path)"
            />
            <div class="flex min-w-0 items-start gap-1 text-fg-subtle tabular-nums">
              <Check v-if="upload.state.phase === 'done'" :size="ICON_PX.in20" class="mt-0.5 shrink-0 text-on-success" />
              <DotLine class="min-w-0" :parts="facts(upload)" />
            </div>
          </template>
        </div>
      </li>
    </ul>
  </section>
</template>
