<script setup lang="ts">
import { computed } from 'vue'
import { Check, CircleX, RotateCw, X } from '@lucide/vue'
import Button from '../ui/Button.vue'
import IconButton from '../ui/IconButton.vue'
import ProgressBar from '../ui/ProgressBar.vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import FileIcon from './FileIcon.vue'
import type { FileUpload, FileUploads } from './file-uploads'
import { formatBytes } from './format'

/**
 * A source's uploads under its file tree, and nothing while there are none:
 * the one on its way first, then those waiting, the failed ones, and the
 * completed ones last, each group in the order it was asked for. The one on
 * its way shows a bar, how much of the file has gone and how fast; a waiting
 * one its size, a completed one a check, a failed one a cross and why; a
 * row's hover names where its file goes. Cancel stops an upload and drops it,
 * Retry sends a failed one again, and Clear drops the finished ones, which
 * can also be dismissed one by one.
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
        <FileIcon :name="upload.file.name" :is-directory="false" />
        <div class="truncate text-chrome leading-5 text-fg-body" :title="upload.path">{{ upload.file.name }}</div>
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
          <template v-if="upload.state.phase === 'uploading'">
            <ProgressBar class="my-1" :value="upload.state.sent" :max="upload.file.size" :label="upload.file.name" />
            <div class="truncate text-fg-subtle tabular-nums">
              {{ formatBytes(upload.state.sent) }} / {{ formatBytes(upload.file.size) }}<template v-if="upload.state.rate !== null"> · {{ formatBytes(upload.state.rate) }}/s</template>
            </div>
          </template>
          <div v-else-if="upload.state.phase === 'waiting'" class="truncate text-fg-subtle">
            Waiting · {{ formatBytes(upload.file.size) }}
          </div>
          <div v-else-if="upload.state.phase === 'done'" class="flex min-w-0 items-center gap-1 text-fg-subtle">
            <Check :size="ICON_PX.in20" class="shrink-0 text-on-success" />
            <span class="truncate">Completed</span>
          </div>
          <!-- The mark sits on the reason's first line, however many it takes. -->
          <div v-else class="flex min-w-0 items-start gap-1 text-on-danger">
            <CircleX :size="ICON_PX.in20" class="mt-0.5 shrink-0" />
            <span class="line-clamp-2">{{ upload.state.message }}</span>
          </div>
        </div>
      </li>
    </ul>
  </section>
</template>
