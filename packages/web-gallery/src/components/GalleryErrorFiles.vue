<script setup lang="ts">
import FileBrowser from '@demicodes/web-ui/files/FileBrowser.vue'
import {
  FileBrowserError,
  type FileBrowserFailure,
  type FileBrowserSource,
} from '@demicodes/web-ui/files/types'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * The file browser's failures, pinned. A directory that cannot be read is a
 * region failure: the list is replaced by the RegionStatus-style pane with an
 * icon per kind. A folder that cannot be created is an action failure: the
 * list stays, InlineError takes the status bar.
 */
const failures: {
  kind: FileBrowserFailure['kind']
  label: string
  home: string
  message: string
}[] = [
  {
    kind: 'offline',
    label: 'Device offline',
    home: '/workspace',
    message: 'The selected device is offline. Reconnect it to browse files.',
  },
  {
    kind: 'permission',
    label: 'Permission denied',
    home: '/workspace/private',
    message: 'This account cannot read /workspace/private.',
  },
  {
    kind: 'not-found',
    label: 'Directory removed',
    home: '/workspace/deleted',
    message: 'The directory /workspace/deleted no longer exists.',
  },
  {
    kind: 'other',
    label: 'Unexpected read failure',
    home: '/workspace',
    message:
      'The directory service returned an incomplete response. Request preview-files-27 could not be completed.',
  },
]
function failingSource(failure: (typeof failures)[number]): FileBrowserSource {
  return {
    platform: 'linux',
    home: failure.home,
    async list(_path, signal) {
      signal?.throwIfAborted()
      throw new FileBrowserError(failure.kind, failure.message)
    },
  }
}
const readOnlySource: FileBrowserSource = {
  platform: 'linux',
  home: '/workspace',
  async list(_path, signal) {
    signal?.throwIfAborted()
    return [
      { name: 'src', isDirectory: true },
      { name: 'README.md', isDirectory: false, size: 1280 },
    ]
  },
  async createDirectory(_path, signal) {
    signal?.throwIfAborted()
    throw new FileBrowserError(
      'permission',
      'Could not create the folder: this workspace is read-only.',
    )
  },
}
</script>

<template>
  <div class="space-y-8">
    <GallerySection
      title="Directory read failures · region"
      note="The list is replaced by a pane with an icon per kind, the sentence for that kind and the source's own message under it."
    >
      <div class="grid gap-6 lg:grid-cols-2">
        <GallerySpecimen
          v-for="failure in failures"
          :key="failure.kind"
          wide
          :variant="failure.label"
        >
          <div class="h-[22rem] overflow-hidden rounded-xl border border-line">
            <FileBrowser :source="failingSource(failure)" mode="file" />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>
    <GallerySection
      title="Folder creation failure · action"
      note="Choose New folder and confirm a name. The source always rejects it, so the list stays and InlineError takes the status bar."
    >
      <div class="h-[22rem] overflow-hidden rounded-xl border border-line">
        <FileBrowser :source="readOnlySource" mode="directory" />
      </div>
    </GallerySection>
  </div>
</template>
