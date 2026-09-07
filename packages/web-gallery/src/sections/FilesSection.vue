<script setup lang="ts">
import { computed, ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import FileBrowser from '@demicodes/web-ui/files/FileBrowser.vue'
import FileBrowserDialog from '@demicodes/web-ui/files/FileBrowserDialog.vue'
import FileIcon from '@demicodes/web-ui/files/FileIcon.vue'
import { createMemoryFileSource, dir, file } from '@demicodes/web-ui/files/memory-source'
import type { FileBrowserMode, FileBrowserSource } from '@demicodes/web-ui/files/types'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import { createGalleryFileHosts, laptopTree } from '../fixtures/files'

const anatomy: [string, string][] = [
  ['Shape', 'The Windows open dialog: a plain title bar; Back, Forward and Up, the device, the path from its root, then New folder and hidden files as icons; places down the left; a detail list; a status row with the confirm button.'],
  ['Device', 'Its own control before the path: a menu of the devices the caller offers with their online dot. Choosing one asks the caller for that device\'s source and the browser starts over at its home.'],
  ['Address', 'Crumbs from the root, each a jump; deep paths fold their middle into an ellipsis. Clicking the free space turns the bar into a text field with the full path.'],
  ['Icons', 'Every row carries its Material Icon Theme glyph, chosen by name: src and .git as typed folders, package.json with the Node badge, App.vue with the Vue mark. Until the theme loads, and for a name it does not know, a plain outline.'],
  ['List', 'Name, date and size, folders first, names in natural order; a header click sorts, a second click flips. A click selects, a double click or Enter opens a folder or confirms a file. Files show dimmed in folder mode and cannot be picked.'],
  ['Keys', 'Arrows move the selection, Home and End jump, Backspace goes up, ⌥← and ⌥→ walk the history.'],
  ['Status row', 'Says what is selected, as "Selected folder:" or "Selected file:" with the name, or what can be. Right: the confirm button and Cancel. A failure to create a folder reads here.'],
  ['New folder', 'A row at the top of the list with the name ready to type over; Enter creates it and selects it. Only a source that can create directories offers the icon.'],
  ['States', 'A slow device shows a spinner, an empty folder says so, and a folder that cannot be read explains why: missing, locked, or the device offline.'],
]

const iconSamples: [string, boolean][] = [
  ['src', true], ['.git', true], ['node_modules', true], ['docs', true], ['Projects', true],
  ['package.json', false], ['README.md', false], ['index.ts', false], ['app.test.ts', false], ['App.vue', false],
  ['main.rs', false], ['main.go', false], ['app.py', false], ['config.yaml', false], ['logo.png', false], ['LICENSE', false], ['notes', false],
]

const hosts = createGalleryFileHosts()
const hostOptions = hosts.map(({ id, label, online, icon }) => ({ id, label, online, icon }))

// Select folder: the workspace picker's use, opening on the laptop's projects.
const folderHostId = ref('mac')
const folderHost = computed(() => hosts.find((host) => host.id === folderHostId.value)!)
const folderChosen = ref<string | null>(null)
const folderKey = ref(0)

// Open file: the composer's remote attachment, opening inside a project.
const fileHostId = ref('mac')
const fileHost = computed(() => hosts.find((host) => host.id === fileHostId.value)!)
const fileChosen = ref<string | null>(null)
const fileKey = ref(0)

// One browser, inline, switched between the states a folder can be in.
const stateOptions = [
  { value: 'slow', label: 'Slow device' },
  { value: 'empty', label: 'Empty' },
  { value: 'locked', label: 'Locked' },
  { value: 'missing', label: 'Missing' },
  { value: 'offline', label: 'Offline' },
  { value: 'readonly', label: 'Read-only' },
] as const
type StateOption = (typeof stateOptions)[number]['value']
const state = ref<StateOption>('empty')
const laptop = laptopTree()
const stateSources: Record<StateOption, { source: FileBrowserSource; path: string }> = {
  slow: { source: createMemoryFileSource({ home: '/Users/zan', root: laptop, latencyMs: 60_000 }), path: '/Users/zan/Projects' },
  empty: { source: createMemoryFileSource({ home: '/Users/zan', root: laptop }), path: '/Users/zan/Library/Preferences' },
  locked: { source: createMemoryFileSource({ home: '/Users/zan', root: laptop }), path: '/Users/zan/.ssh' },
  missing: { source: createMemoryFileSource({ home: '/Users/zan', root: laptop }), path: '/Users/zan/Projects/gone' },
  offline: { source: hosts[2]!.source, path: '/home/zan' },
  readonly: {
    source: (() => {
      const { createDirectory: _omit, ...rest } = createMemoryFileSource({ home: '/srv', root: dir({ srv: dir({ 'release.tar.gz': file(90_211_004, '2026-09-01T08:00:00Z') }) }) })
      return rest
    })(),
    path: '/srv',
  },
}
const stateMode = ref<FileBrowserMode>('directory')
const modeOptions = [
  { value: 'directory', label: 'Folder' },
  { value: 'file', label: 'File' },
] as const

function selectHost(target: 'folder' | 'file', id: string) {
  if (target === 'folder') {
    folderHostId.value = id
    folderKey.value += 1
  } else {
    fileHostId.value = id
    fileKey.value += 1
  }
}
</script>

<template>
  <div class="flex flex-col gap-10">
    <GallerySection title="Files" note="One browser for choosing a folder on a device and for opening a file there. Fixture trees; nothing reads a disk.">
      <dl class="grid max-w-3xl grid-cols-[7rem_minmax(0,1fr)] gap-x-4 gap-y-2 text-[13px] leading-5">
        <template v-for="[term, detail] in anatomy" :key="term">
          <dt class="select-none text-fg-subtle">{{ term }}</dt>
          <dd class="text-fg-muted">{{ detail }}</dd>
        </template>
      </dl>
    </GallerySection>

    <GallerySection title="Icons" note="The Material Icon Theme, resolved by name; the manifest and each glyph load on first use.">
      <div class="flex flex-wrap gap-x-6 gap-y-2">
        <span v-for="[name, isDirectory] in iconSamples" :key="name" class="flex select-none items-center gap-2 text-chrome text-fg-body">
          <FileIcon :name="name" :is-directory="isDirectory" />{{ name }}
        </span>
      </div>
    </GallerySection>

    <GallerySection title="Select folder" note="Creating or moving a workspace: the dialog opens at the device's projects, with the recent workspaces as places. Choose another device in the address bar; the offline one shows what that looks like.">
      <GalleryOverlayWell size="tall">
        <FileBrowserDialog
          :key="folderKey"
          :is-open="true"
          :overlay-store="appOverlayStore"
          mode="directory"
          title="Select folder"
          :source="folderHost.source"
          :initial-path="folderHostId === 'mac' ? '/Users/zan/Projects' : undefined"
          :places="folderHost.places"
          :hosts="hostOptions"
          :host-id="folderHostId"
          @select="folderChosen = $event"
          @update:host-id="selectHost('folder', $event)"
        />
      </GalleryOverlayWell>
      <p class="select-none text-[12px] text-fg-subtle">
        Chosen: <span class="select-text text-fg-muted">{{ folderChosen ?? '—' }}</span>
      </p>
    </GallerySection>

    <GallerySection title="Open file" note="The composer's remote attachment: opens inside the conversation's workspace. Folders are entered, a file is the answer.">
      <GalleryOverlayWell size="tall">
        <FileBrowserDialog
          :key="fileKey"
          :is-open="true"
          :overlay-store="appOverlayStore"
          mode="file"
          title="Open file"
          :source="fileHost.source"
          :initial-path="fileHostId === 'mac' ? '/Users/zan/Projects/demi' : undefined"
          :places="fileHost.places"
          :hosts="hostOptions"
          :host-id="fileHostId"
          @select="fileChosen = $event"
          @update:host-id="selectHost('file', $event)"
        />
      </GalleryOverlayWell>
      <p class="select-none text-[12px] text-fg-subtle">
        Chosen: <span class="select-text text-fg-muted">{{ fileChosen ?? '—' }}</span>
      </p>
    </GallerySection>

    <GallerySection title="States" note="The browser alone, without the dialog and without a sidebar, in each state a folder can be in.">
      <div class="mb-3 flex flex-wrap items-center gap-3">
        <Segmented v-model="state" :options="stateOptions" />
        <Segmented v-model="stateMode" :options="modeOptions" />
      </div>
      <GallerySpecimen wide>
        <div class="gallery-frame flex h-96 w-full max-w-3xl flex-col overflow-hidden">
          <FileBrowser
            :key="`${state}-${stateMode}`"
            :mode="stateMode"
            :source="stateSources[state].source"
            :initial-path="stateSources[state].path"
          />
        </div>
      </GallerySpecimen>
    </GallerySection>

    <GallerySection title="Narrow" note="At a phone width the path takes its own row under the toolbar, the places become a menu at the toolbar's right, Forward and the date column go, and the address bar folds.">
      <GalleryOverlayWell size="narrow">
        <FileBrowserDialog
          :is-open="true"
          :overlay-store="appOverlayStore"
          mode="directory"
          :source="hosts[0]!.source"
          initial-path="/Users/zan/Projects/a project with a very long directory name that will not fit in the address bar/src"
          :places="hosts[0]!.places"
        />
      </GalleryOverlayWell>
    </GallerySection>
  </div>
</template>
