<script setup lang="ts">
import { computed } from 'vue'
import { Check, RotateCw, X } from '@lucide/vue'
import Button from '../ui/Button.vue'
import IconButton from '../ui/IconButton.vue'
import ProgressBar from '../ui/ProgressBar.vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import FileIcon from './FileIcon.vue'
import type { FileUploadView } from './file-uploads'
import { formatBytes } from './format'
import { baseName, parentPath, relativePath } from './paths'

/**
 * The uploads under a file tree, in the order they were asked for. The one on
 * its way shows a bar and how much of the file has gone, the rest wait their
 * turn, a landed one says where it went, and a failed one why. Cancel stops
 * an upload and drops it; Retry sends a failed one again; Clear drops the
 * finished ones, and a finished one can be dismissed alone.
 */
const props = defineProps<{
  uploads: readonly FileUploadView[]
  /** The workspace, to say where a file goes from it. */
  root: string
  /** What the workspace is called in place of its directory's name. */
  rootName?: string
}>()

const emit = defineEmits<{
  cancel: [id: string]
  retry: [id: string]
  dismiss: [id: string]
  clear: []
}>()

/** Waiting or on its way: what Cancel stops, where a finished upload is dismissed instead. */
function underWay(upload: FileUploadView): boolean {
  return upload.state.phase === 'waiting' || upload.state.phase === 'uploading'
}

const finished = computed(() => props.uploads.some((upload) => !underWay(upload)))

/** The directory a file goes into, as it reads from the workspace. */
function destination(upload: FileUploadView): string {
  return relativePath(props.root, parentPath(upload.path)) || props.rootName || baseName(props.root) || '/'
}
</script>

<template>
  <section class="flex max-h-60 shrink-0 flex-col border-t border-line bg-surface-editor" aria-label="Uploads">
    <div class="flex h-8 shrink-0 select-none items-center pl-3 pr-1 text-chrome font-medium text-fg-muted">
      <span class="min-w-0 flex-1 truncate">Uploads</span>
      <Button v-if="finished" size="sm" variant="ghost" class="shrink-0" @click="emit('clear')">Clear</Button>
    </div>
    <ul class="min-h-0 overflow-y-auto px-1 pb-1">
      <li
        v-for="upload in uploads"
        :key="upload.id"
        class="flex items-start gap-1.5 rounded-md py-1 pl-2 pr-1"
      >
        <FileIcon :name="upload.file.name" :is-directory="false" class="mt-0.5" />
        <div class="min-w-0 flex-1">
          <div class="truncate text-chrome leading-5 text-fg-body" :title="upload.path">{{ upload.file.name }}</div>
          <div
            v-if="upload.state.phase === 'uploading'"
            class="flex items-center gap-2 text-[11px] leading-4 text-fg-subtle tabular-nums"
          >
            <ProgressBar class="min-w-0 flex-1" :value="upload.state.sent" :max="upload.file.size" :label="upload.file.name" />
            <span class="shrink-0">{{ formatBytes(upload.state.sent) }} / {{ formatBytes(upload.file.size) }}</span>
          </div>
          <div v-else-if="upload.state.phase === 'waiting'" class="truncate text-[11px] leading-4 text-fg-subtle">
            Waiting · {{ formatBytes(upload.file.size) }}
          </div>
          <div v-else-if="upload.state.phase === 'done'" class="flex min-w-0 items-center gap-1 text-[11px] leading-4 text-fg-subtle">
            <Check :size="ICON_PX.in20" class="shrink-0 text-on-success" />
            <span class="truncate">In {{ destination(upload) }}</span>
          </div>
          <div v-else class="line-clamp-2 text-[11px] leading-4 text-on-danger">
            {{ upload.state.message }}
          </div>
        </div>
        <Tooltip v-if="upload.state.phase === 'failed'" content="Retry" class="shrink-0">
          <IconButton :icon="RotateCw" size="xs" variant="ghost" aria-label="Retry" spin-on-click @click="emit('retry', upload.id)" />
        </Tooltip>
        <Tooltip :content="underWay(upload) ? 'Cancel' : 'Dismiss'" class="shrink-0">
          <IconButton
            :icon="X"
            size="xs"
            variant="ghost"
            :aria-label="underWay(upload) ? 'Cancel' : 'Dismiss'"
            @click="underWay(upload) ? emit('cancel', upload.id) : emit('dismiss', upload.id)"
          />
        </Tooltip>
      </li>
    </ul>
  </section>
</template>
