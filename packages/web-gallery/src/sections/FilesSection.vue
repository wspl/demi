<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, shallowRef, watch } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import FileBrowser from '@demicodes/web-ui/files/FileBrowser.vue'
import FileBrowserAddressBar from '@demicodes/web-ui/files/FileBrowserAddressBar.vue'
import FileBrowserDialog from '@demicodes/web-ui/files/FileBrowserDialog.vue'
import WorkspaceDialog from '@demicodes/web-ui/hosts/WorkspaceDialog.vue'
import type { WorkspaceDraft, WorkspaceProject } from '@demicodes/web-ui/hosts/workspace'
import FileIcon from '@demicodes/web-ui/files/FileIcon.vue'
import FileTree from '@demicodes/web-ui/files/FileTree.vue'
import UploadConflictDialog from '@demicodes/web-ui/files/UploadConflictDialog.vue'
import { uploadsOf, type FileUploads, type UploadClash, type UploadItem, type UploadPlacement } from '@demicodes/web-ui/files/file-uploads'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { TREE_ROOT, TREE_SELECTED, failingSource, offlineSource, rowsSource, sizedFile, sizedFolder, stuckSource, uploadSource } from '../fixtures/file-trees'
import { createMemoryFileSource, dir, file } from '@demicodes/web-ui/files/memory-source'
import type { FileBrowserMode, FileBrowserSource } from '@demicodes/web-ui/files/types'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import GalleryDialogFrame from '../components/GalleryDialogFrame.vue'
import { productWould } from '../product-would'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import { createGalleryRemoteFileHosts, laptopTree } from '../fixtures/files'
import { createGalleryWorkspace } from '../fixtures/workspace'
import { useGalleryView } from '../gallery-views'
import { baseName } from '@demicodes/web-ui/files/paths'

const anatomy: [string, string][] = [
  [
    'Shape',
    'The Windows open dialog: a plain title bar; Back, Forward and Up, the device, the path from its root, then New folder and hidden files as icons; places down the left; a detail list; a status row with the confirm button.'
  ],
  [
    'Device',
    'Its own control before the path: a menu of the devices the caller offers with their online dot. Choosing one asks the caller for that device\'s source and the browser starts over at its home.'
  ],
  [
    'Address',
    'Crumbs from the root, each a jump. A bar too narrow for every name turns crumbs into their glyphs, one at a time from the left, each name kept in a tooltip, so the crumbs nearest the end keep their names longest; when even the glyphs overflow, the bar keeps its right end and clips its left. A click anywhere on the bar but a crumb turns it into a text field with the full path, and Enter goes there.'
  ],
  [
    'Icons',
    'Every row, crumb, place and the status entry carry a Material Icon Theme glyph, chosen by name: src and .git as typed folders, package.json with the Node badge, App.vue with the Vue mark. Until the theme loads, and for a name it does not know, a plain outline.'
  ],
  [
    'List',
    'Name, date and size, folders first, names in VS Code\'s order: case ignored, numbers by value, punctuation first, so hidden entries lead and show faded; a header click sorts, a second click flips. A click selects, a double click or Enter opens a folder or confirms a file. Files show dimmed in folder mode and cannot be picked.'
  ],
  [
    'Keys',
    'Arrows move the selection, Home and End jump, Backspace goes up, ⌥← and ⌥→ walk the history.'
  ],
  [
    'Status row',
    'Says what is selected, as "Selected folder:" or "Selected file:" with the name, or what can be. Right: the confirm button and Cancel. A failure to create a folder reads here.'
  ],
  [
    'New project',
    'The working-environment dialog: Device or Cloud as two cards, Device first; a device asks which one, with Add device beside the menu, and a directory on it, the project named after the folder; the Cloud only asks a name; Browse… turns the dialog into the folder browser. Switching between projects is the sidebar\'s Move to and the header\'s workspace control, not a dialog.'
  ],
  [
    'New folder',
    'A row at the top of the list with the name ready to type over; Enter creates it and selects it. Only a source that can create directories offers the icon.'
  ],
  [
    'States',
    'A slow device shows a spinner, an empty folder says so, and a folder that cannot be read explains why: missing, locked, or the device offline.'
  ],
]

const iconSamples: [string, boolean][] = [
  ['src', true], ['.git', true], ['node_modules', true], ['docs', true], ['Projects', true],
  ['package.json', false], ['README.md', false], ['index.ts', false], ['app.test.ts', false], ['App.vue', false],
  ['main.rs', false], ['main.go', false], ['app.py', false], ['config.yaml', false], ['logo.png', false], ['LICENSE', false], ['notes', false],
]

const hosts = createGalleryRemoteFileHosts()
const pairedHosts = hosts.filter((host) => !host.canWake)
const { view } = useGalleryView()

// New project: the working-environment dialog over the same hosts; a created project joins the list.
const workspaceDevices = ref(pairedHosts.map(({ id, label, online }) => ({ id, name: label, online })))
/** Add device stands in for the pairing flow: a new online device joins the list. */
function connectWorkspaceDevice() {
  workspaceDevices.value.push(
    {
      id: `device-${Date.now()}`,
      name: `host-${workspaceDevices.value.length + 1}`,
      online: true
    }
  )
}
const workspaceProjects = ref<WorkspaceProject[]>([
  {
    id: 'demi',
    name: 'demi',
    host: 'zan-mbp',
    path: '/Users/zan/Projects/demi'
  },
  {
    id: 'assets',
    name: 'assetsfactory',
    host: 'build-01',
    path: '/srv/assetsfactory'
  },
])
const workspaceMessage = ref('')
const workspaceKey = ref(0)
const cloudSource = createMemoryFileSource(
  {
    platform: 'linux',
    home: '/home/demi',
    root: dir({
      home: dir({
        demi: dir({ workspace: dir({}), scratch: dir({}) })
      })
    })
  }
)
const sourceFor = (id: string) =>
  (id === 'cloud'
  ? cloudSource
  : (hosts.find((host) => host.id === id) ?? hosts[0]!).source)
const placesFor = (id: string) => (id === 'cloud' ? [
  {
    label: 'Quick access',
    places: [{ path: '/home/demi', label: 'Home' }]
  }
] : (hosts.find((host) => host.id === id) ?? hosts[0]!).places)
function createWorkspace(draft: WorkspaceDraft) {
  const name = draft.kind === 'cloud'
    ? draft.name
    : baseName(draft.path) || 'Workspace'
  if (workspaceProjects.value.some((project) => project.name === name)) {
    workspaceMessage.value = `A project called ${name} already exists.`
    return
  }
  workspaceMessage.value = ''
  const host = draft.kind === 'cloud'
    ? 'Cloud'
    : workspaceDevices.value.find((device) => device.id === draft.deviceId)?.name ??
    draft.deviceId
  workspaceProjects.value.push(
    {
      id: `p-${Date.now()}`,
      name,
      host,
      path: draft.kind === 'cloud' ? `/home/demi/${name}` : draft.path
    }
  )
  workspaceKey.value += 1
}

// Select folder: the workspace picker's use, opening on the laptop's projects.
const folderHostId = ref('mac')
const folderHost = computed(() => hosts.find((host) => host.id === folderHostId.value)!)
const folderChosen = ref<string | null>(null)
const folderKey = ref(0)

// Open file: the composer's remote attachment, opening inside a project.
const fileHostId = ref('mac')
const fileHost = computed(() => hosts.find((host) => host.id === fileHostId.value)!)
// The address bar at shrinking widths: a file deep in the workspace, as the File view shows it.
const addressWorkspace = createGalleryWorkspace()
const addressPath = `${addressWorkspace.root}/packages/web-ui/src/files/FileBrowserAddressBar.vue`
const addressWidths = [
  { variant: 'every name', width: '40rem' },
  { variant: 'names go from the left', width: '28rem' },
  { variant: 'glyphs only', width: '15rem' },
  { variant: 'the left clipped', width: '9rem' },
] as const
const narrowHostId = ref(hosts[0]!.id)
const narrowHost = computed(() => hosts.find((host) => host.id === narrowHostId.value)!)
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
const stateSources: Record<StateOption, {
  source: FileBrowserSource;
  path: string
}> = {
  slow: {
    source: createMemoryFileSource(
      {
        platform: 'linux',
        home: '/Users/zan',
        root: laptop,
        latencyMs: 60_000
      }
    ),
    path: '/Users/zan/Projects'
  },
  empty: {
    source: createMemoryFileSource({
      platform: 'linux',
      home: '/Users/zan',
      root: laptop
    }),
    path: '/Users/zan/Library/Preferences'
  },
  locked: {
    source: createMemoryFileSource({
      platform: 'linux',
      home: '/Users/zan',
      root: laptop
    }),
    path: '/Users/zan/.ssh'
  },
  missing: {
    source: createMemoryFileSource({
      platform: 'linux',
      home: '/Users/zan',
      root: laptop
    }),
    path: '/Users/zan/Projects/gone'
  },
  offline: { source: hosts[2]!.source, path: '/home/zan' },
  readonly: {
    source: (() => {
      const { createDirectory: _omit, ...rest } = createMemoryFileSource(
        {
          platform: 'linux',
          home: '/srv',
          root: dir(
            {
              srv: dir({
                'release.tar.gz': file(90_211_004, '2026-09-01T08:00:00Z')
              })
            }
          )
        }
      )
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

// The Tree view: one source per specimen, and the pinned-path specimens set up scrolled.
const treeRows = rowsSource()

const MiB = 1024 * 1024

/** `count` files of about `size` bytes each, named `name-01.ext` on. */
function numbered(name: string, ext: string, count: number, size: number): [string, number][] {
  return Array.from({ length: count }, (_, index) => [`${name}-${String(index + 1).padStart(2, '0')}.${ext}`, size + index * 97_000])
}

/**
 * A workspace with an upload in each state, at sizes real ones have: files
 * and a folder landed, a file refused by `src/auth`, a folder merged into
 * `src` whose files there were refused too, a folder of screenshots on its
 * way and two large files waiting behind it.
 */
function seededUploads(): { source: FileBrowserSource; uploads: FileUploads } {
  const source = uploadSource()
  const uploads = uploadsOf(source)
  const add = (directory: string, item: UploadItem, placement: UploadPlacement = 'add') =>
    uploads.add(directory, [{ item, placement }])
  add(`${TREE_ROOT}/src`, { kind: 'file', file: sizedFile('logo.svg', 4_096) })
  add(TREE_ROOT, sizedFolder('assets', ['icons'], [['icons/add.svg', 1_024], ['icons/close.svg', 980], ['banner.png', 182_000]]))
  add(TREE_ROOT, { kind: 'file', file: sizedFile('notes.md', 2_310) })
  add(`${TREE_ROOT}/src/auth`, { kind: 'file', file: sizedFile('model.bin', 90 * MiB) })
  add(TREE_ROOT, sizedFolder('src', ['auth', 'http'], [
    ['index.ts', 910],
    ['auth/token.ts', 2_140],
    ['auth/scopes.ts', 1_320],
    ['http/client.ts', 4_010],
  ]), 'overwrite')
  add(`${TREE_ROOT}/docs`, sizedFolder('screenshots', [], numbered('screenshot', 'png', 24, 14 * MiB)))
  add(`${TREE_ROOT}/docs`, { kind: 'file', file: sizedFile('demo-recording.mov', 480 * MiB) })
  add(`${TREE_ROOT}/docs`, { kind: 'file', file: sizedFile('dataset.parquet', Math.round(1.2 * 1024 * MiB)) })
  return { source, uploads }
}

const pinnedUploads = shallowRef(seededUploads())
const uploadTree = ref<InstanceType<typeof FileTree> | null>(null)

/** Stops everything the specimen has waiting or on its way. */
function stopPinnedUploads(): void {
  const { uploads } = pinnedUploads.value
  for (const item of [...uploads.items])
    uploads.cancel(item.id)
}

function resetPinnedUploads(): void {
  stopPinnedUploads()
  pinnedUploads.value = seededUploads()
}

onBeforeUnmount(stopPinnedUploads)

// What the drop buttons hand the tree, as a drop from the desktop would.
function dropPhotos(): void {
  void uploadTree.value?.upload(`${TREE_ROOT}/docs`, [
    sizedFolder('photos', ['2025', '2025/raw'], [
      ...numbered('2025/raw/IMG', 'heic', 6, 3 * MiB),
      ...numbered('2025/IMG', 'jpg', 6, 2 * MiB),
      ['cover.jpg', 4 * MiB],
    ]),
  ])
}

/** Two names the workspace has, a folder and a file, and one it has not: the question with Merge. */
function dropOnWorkspace(): void {
  void uploadTree.value?.upload(TREE_ROOT, [
    sizedFolder('src', ['http'], [['index.ts', 950], ['http/cors.ts', 1_830]]),
    { kind: 'file', file: sizedFile('README.md', 3_072) },
    { kind: 'file', file: sizedFile('SECURITY.md', 1_536) },
  ])
}

// The replace questions: each answer shows beside its own, and Show asks it again.
const questions: { variant: string; clashes: UploadClash[]; directory: string; others: { files: number; folders: number } }[] = [
  {
    variant: 'files: Replace or Skip',
    clashes: [
      { name: 'logo.svg', isDirectory: false, takenByDirectory: false },
      { name: 'photo.png', isDirectory: false, takenByDirectory: false },
    ],
    directory: 'src',
    others: { files: 1, folders: 0 },
  },
  {
    variant: 'a folder meets a folder: Merge too',
    clashes: [{ name: 'photos', isDirectory: true, takenByDirectory: true }],
    directory: 'docs',
    others: { files: 0, folders: 0 },
  },
  {
    variant: 'a folder and a file',
    clashes: [
      { name: 'src', isDirectory: true, takenByDirectory: true },
      { name: 'README.md', isDirectory: false, takenByDirectory: false },
    ],
    directory: 'demi',
    others: { files: 1, folders: 0 },
  },
]
const questionState = reactive(questions.map(() => ({ open: true, answer: null as string | null })))

function answerQuestion(index: number, answer: string): void {
  questionState[index]!.answer = answer
  questionState[index]!.open = false
}

// The drop states, held still: a drag over the empty space, a folder, a
// file, a closed folder, and a folder whose row has scrolled under the stack.
const dropPinned = ref<InstanceType<typeof FileTree> | null>(null)
const treeStuckRoot = stuckSource(TREE_ROOT)
const treeStuckDir = stuckSource(`${TREE_ROOT}/src/http`)
const treeFailing = failingSource()
const treeOffline = offlineSource()
const treeSelected = ref<string | null>(TREE_SELECTED)
type TreeHandle = { scrollToRow: (path: string) => void; scrollBy: (px: number) => void }
const pinnedTwo = ref<TreeHandle | null>(null)
const pinnedLeaving = ref<TreeHandle | null>(null)
const pinnedPast = ref<TreeHandle | null>(null)

/** Once the listings are in, scroll each pinned-path specimen to its state. */
async function scrollPinnedSpecimens() {
  await nextTick()
  await new Promise((resolve) => setTimeout(resolve, 50))
  // Path pinned: the directories down to oauth all scrolled out above it.
  pinnedTwo.value?.scrollToRow(`${TREE_ROOT}/src/auth/providers/oauth`)
  // Leaving: github's last row half under the stack, so the deepest pinned row slides.
  pinnedLeaving.value?.scrollToRow(`${TREE_ROOT}/src/auth/providers/oauth/github`)
  pinnedLeaving.value?.scrollBy(3 * 29 + 14)
  // Past: everything in src scrolled out, nothing pins.
  pinnedPast.value?.scrollToRow(`${TREE_ROOT}/tests`)
  // A drop into oauth, its row scrolled under the stack like the path pinned above.
  dropPinned.value?.scrollToRow(`${TREE_ROOT}/src/auth/providers/oauth`)
}
watch(view, (next) => {
  if (next === 'tree') {
    void scrollPinnedSpecimens()
  }
})
onMounted(() => {
  if (view.value === 'tree') {
    void scrollPinnedSpecimens()
  }
})
</script>

<template>
  <div class="flex flex-col gap-10">
    <template v-if="view === 'browser'">
      <GallerySection
        title="Files"
        note="One browser for choosing a folder on a device and for opening a file there. Fixture trees; nothing reads a disk."
      >
        <dl
          class="grid max-w-3xl grid-cols-[7rem_minmax(0,1fr)] gap-x-4 gap-y-2 text-[13px] leading-5"
        >
          <template v-for="[term, detail] in anatomy" :key="term">
            <dt class="select-none text-fg-subtle">{{ term }}</dt>
            <dd class="text-fg-muted">{{ detail }}</dd>
          </template>
        </dl>
      </GallerySection>

      <GallerySection
        title="Icons"
        note="The Material Icon Theme, resolved by name; the manifest and each glyph load on first use."
      >
        <div class="flex flex-wrap gap-x-6 gap-y-2">
          <span
            v-for="[name, isDirectory] in iconSamples"
            :key="name"
            class="flex select-none items-center gap-2 text-chrome text-fg-body"
          >
            <FileIcon :name="name" :is-directory="isDirectory" />{{ name }}
          </span>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'tree'">
      <GallerySection
        title="Rows"
        note="The workspace tree: open and closed directories, files, the selected file, deep nesting, long names truncated. Directories come before files and each sorts as VS Code's explorer does, so hidden entries lead, drawn faded. Click a directory to fold it, a file to select it; the control at the caption's end lists the open directories again."
      >
        <GallerySpecimen variant="rows · live">
          <div class="gallery-frame h-[28rem] w-[220px] overflow-hidden bg-surface-editor">
            <FileTree :source="treeRows" :root="TREE_ROOT" :selected="treeSelected" @open="treeSelected = $event" />
          </div>
        </GallerySpecimen>
      </GallerySection>
      <GallerySection
        title="Loading and failures"
        note="A workspace still listing shows a spinner in place of rows; a directory still listing keeps its chevron and spins at the row's end. A directory that could not be listed wears a red dot on its icon, the reason as tooltip. A workspace that cannot be listed says so."
      >
        <div class="flex flex-wrap gap-6">
          <GallerySpecimen variant="workspace listing">
            <div class="gallery-frame h-[16rem] w-[220px] overflow-hidden bg-surface-editor">
              <FileTree :source="treeStuckRoot" :root="TREE_ROOT" :selected="null" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="directory listing">
            <div class="gallery-frame h-[16rem] w-[220px] overflow-hidden bg-surface-editor">
              <FileTree :source="treeStuckDir" :root="TREE_ROOT" :selected="`${TREE_ROOT}/src/http/router.ts`" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="no access · unavailable">
            <div class="gallery-frame h-[16rem] w-[220px] overflow-hidden bg-surface-editor">
              <FileTree :source="treeFailing" :root="TREE_ROOT" :selected="`${TREE_ROOT}/src/auth/cookie.ts`" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="workspace unavailable">
            <div class="gallery-frame h-[16rem] w-[220px] overflow-hidden bg-surface-editor">
              <FileTree :source="treeOffline" :root="TREE_ROOT" :selected="null" />
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection
        title="Pinned path"
        note="The workspace name always heads the tree. Its lower fade is absent at scroll position zero and appears only when rows scroll underneath the pinned stack. Under it pin the directories the rows at the top sit in, each once its own row scrolls out above and until its last row has too; the deepest slides out under the ones above. A closed directory never pins. Scroll each tree to move through the states."
      >
        <div class="flex flex-wrap gap-6">
          <GallerySpecimen variant="at the top">
            <div class="gallery-frame h-[16rem] w-[220px] overflow-hidden bg-surface-editor">
              <FileTree :source="rowsSource()" :root="TREE_ROOT" :selected="TREE_SELECTED" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="path pinned">
            <div class="gallery-frame h-[16rem] w-[220px] overflow-hidden bg-surface-editor">
              <FileTree ref="pinnedTwo" :source="rowsSource()" :root="TREE_ROOT" :selected="TREE_SELECTED" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="deepest leaving">
            <div class="gallery-frame h-[16rem] w-[220px] overflow-hidden bg-surface-editor">
              <FileTree ref="pinnedLeaving" :source="rowsSource()" :root="TREE_ROOT" :selected="TREE_SELECTED" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="past, at a closed directory">
            <div class="gallery-frame h-[16rem] w-[220px] overflow-hidden bg-surface-editor">
              <FileTree ref="pinnedPast" :source="rowsSource()" :root="TREE_ROOT" :selected="TREE_SELECTED" />
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection
        title="Drag and drop"
        note="Files and folders dragged in from the desktop upload where they drop, and that place lights under a dashed line while the drag is over it: a folder with the rows it holds, the folder a file sits in, or the whole workspace over the empty space and over a file at the top. A folder whose row has scrolled under the pinned path lights there too. A drag resting on a closed folder opens it after a moment, to drop deeper. Only a host that takes uploads takes drops. These trees hold each state still and take no drops; the workspace under Upload and download takes real ones."
      >
        <div class="flex flex-wrap gap-6">
          <GallerySpecimen variant="over the empty space: the workspace">
            <div class="gallery-frame h-[20rem] w-[220px] overflow-hidden bg-surface-editor">
              <FileTree :source="rowsSource()" :root="TREE_ROOT" :selected="null" :dropping="TREE_ROOT" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="over a folder: it and what it holds">
            <div class="gallery-frame h-[20rem] w-[220px] overflow-hidden bg-surface-editor">
              <FileTree :source="rowsSource()" :root="TREE_ROOT" :selected="`${TREE_ROOT}/src/http/router.ts`" :dropping="`${TREE_ROOT}/src/http`" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="over a file: the folder it sits in">
            <div class="gallery-frame h-[20rem] w-[220px] overflow-hidden bg-surface-editor">
              <FileTree :source="rowsSource()" :root="TREE_ROOT" :selected="`${TREE_ROOT}/src/http/router.ts`" :dropping="`${TREE_ROOT}/src/index.ts`" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="over a closed folder">
            <div class="gallery-frame h-[20rem] w-[220px] overflow-hidden bg-surface-editor">
              <FileTree :source="rowsSource()" :root="TREE_ROOT" :selected="null" :dropping="`${TREE_ROOT}/docs`" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="under the pinned path">
            <div class="gallery-frame h-[20rem] w-[220px] overflow-hidden bg-surface-editor">
              <FileTree
                ref="dropPinned"
                :source="rowsSource()"
                :root="TREE_ROOT"
                :selected="TREE_SELECTED"
                :dropping="`${TREE_ROOT}/src/auth/providers/oauth`"
              />
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection
        title="Upload and download"
        note="A right-click offers what the host can do there: a file downloads; a folder, or the empty space for the workspace itself, takes files uploaded into it. Names the folder already has wait on a question: Replace puts what came in place of what is there, Skip uploads the rest, closing uploads nothing; once a folder meets a folder, Merge adds its files to the one there, writing over files with the same names. The uploads list under the tree, fitting its rows until the divider above it sets its height, one on its way at a time and the rest waiting, a folder as one row. Each row is its name and one line of facts, the one on its way with a bar between them: how much has gone and how fast, and of a folder how many files; Completed once one has landed; why one failed, or of a folder how many of its files did. A line too long for the list is cut and its tooltip shows it whole; the name's shows where it goes, and a folder's failures list in theirs. Cancel stops one, and a folder keeps the files that landed; Retry sends what failed again; Clear drops the finished ones. A folder an upload changes is listed again. The first specimen is a workspace seeded with an upload in each state, src/auth refusing them; every control works, Reset seeds it again, and the drop buttons hand its tree what a drop from the desktop would. Files and folders dropped on its tree, or right-clicked in, upload at a pace slow enough to watch, as they do in the File view (Session, Panel)."
      >
        <div class="flex flex-wrap items-start gap-6">
          <GallerySpecimen variant="uploads · every state, live">
            <!-- The stage lays children out in a row; the frame and its controls stack in their own column. -->
            <div class="flex w-[220px] flex-col gap-2">
              <div class="gallery-frame h-[30rem] w-[220px] overflow-hidden bg-surface-editor">
                <FileTree
                  ref="uploadTree"
                  :source="pinnedUploads.source"
                  :root="TREE_ROOT"
                  :selected="null"
                />
              </div>
              <div class="flex flex-wrap gap-1">
                <Button size="sm" variant="ghost" @click="resetPinnedUploads">Reset</Button>
                <Button size="sm" variant="ghost" @click="dropPhotos">Drop photos/ on docs</Button>
                <Button size="sm" variant="ghost" @click="dropOnWorkspace">Drop src/ + 2 files on the workspace</Button>
              </div>
            </div>
          </GallerySpecimen>
          <GallerySpecimen v-for="(question, index) in questions" :key="question.variant" :variant="`the question · ${question.variant}`">
            <div class="flex flex-col gap-2">
              <GalleryDialogFrame v-if="questionState[index]!.open">
                <UploadConflictDialog
                  :is-open="true"
                  :overlay-store="appOverlayStore"
                  :clashes="question.clashes"
                  :directory="question.directory"
                  :others="question.others"
                  @replace="answerQuestion(index, 'Replace')"
                  @merge="answerQuestion(index, 'Merge')"
                  @skip="answerQuestion(index, 'Skip')"
                  @cancel="answerQuestion(index, 'Closed, nothing uploaded')"
                />
              </GalleryDialogFrame>
              <div v-else>
                <Button size="sm" @click="questionState[index]!.open = true">Show</Button>
              </div>
              <p class="select-none text-[12px] text-fg-subtle">
                Answered: <span class="select-text text-fg-muted">{{ questionState[index]!.answer ?? '—' }}</span>
              </p>
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'dialogs'">
      <GallerySection
        title="Select folder"
        note="Creating or moving a workspace: the dialog opens at the device's projects, with the recent workspaces as places. Choose another device in the address bar; the offline device is listed but cannot be chosen."
      >
        <GalleryDialogFrame v-slot="{ open, close }">
          <FileBrowserDialog
            :key="folderKey"
            :is-open="open"
            :overlay-store="appOverlayStore"
            mode="directory"
            title="Select folder"
            :source="folderHost.source"
            :initial-path="folderHostId === 'mac' ? '/Users/zan/Projects' : undefined"
            :places="folderHost.places"
            :hosts="pairedHosts"
            :host-id="folderHostId"
            @select="folderChosen = $event"
            @update:host-id="selectHost('folder', $event)"
            @close="close"
          />
        </GalleryDialogFrame>
        <p class="select-none text-[12px] text-fg-subtle">
        Chosen: <span class="select-text text-fg-muted">{{ folderChosen ?? '—' }}</span>
        </p>
      </GallerySection>

      <GallerySection
        title="Open file"
        note="The composer's remote attachment: opens inside the conversation's workspace. Folders are entered, a file is the answer. Cloud shows no status and remains selectable while asleep; its source wakes it when browsing."
      >
        <GalleryDialogFrame v-slot="{ open, close }">
          <FileBrowserDialog
            :key="fileKey"
            :is-open="open"
            :overlay-store="appOverlayStore"
            mode="file"
            title="Open file"
            :source="fileHost.source"
            :initial-path="fileHostId === 'mac' ? '/Users/zan/Projects/demi' : undefined"
            :places="fileHost.places"
            :hosts="hosts"
            :host-id="fileHostId"
            @select="fileChosen = $event"
            @update:host-id="selectHost('file', $event)"
            @close="close"
          />
        </GalleryDialogFrame>
        <p class="select-none text-[12px] text-fg-subtle">
        Chosen: <span class="select-text text-fg-muted">{{ fileChosen ?? '—' }}</span>
        </p>
      </GallerySection>

      <GallerySection
        title="New project"
        note="The working-environment dialog on its form: Device or Cloud. A device asks which one (Add device after the menu stands in for pairing) and a directory; the Cloud only asks a name, the project named after the folder, with Browse… turning the dialog into the folder browser. A name already in the list is refused under the form."
      >
        <GalleryDialogFrame v-slot="{ open, close }" class="max-w-md">
          <WorkspaceDialog
            :key="workspaceKey"
            :is-open="open"
            :overlay-store="appOverlayStore"
            :devices="workspaceDevices"
            :message="workspaceMessage"
            :source-for="sourceFor"
            :places-for="placesFor"
            @create="createWorkspace"
            @connect-device="connectWorkspaceDevice"
            @close="close"
          />
        </GalleryDialogFrame>
        <p class="select-none text-[12px] text-fg-subtle">
        Projects: <span class="select-text text-fg-muted">{{ workspaceProjects.map((project) => project.name).join(', ') }}</span>
        </p>
      </GallerySection>
    </template>

    <template v-if="view === 'browser'">
      <GallerySection
        title="States"
        note="The browser alone, without the dialog and without a sidebar, in each state a folder can be in."
      >
        <div class="mb-3 flex flex-wrap items-center gap-3">
          <Segmented v-model="state" :options="stateOptions" />
          <Segmented v-model="stateMode" :options="modeOptions" />
        </div>
        <GallerySpecimen wide>
          <div
            class="gallery-frame flex h-96 w-full max-w-3xl flex-col overflow-hidden"
          >
            <FileBrowser
              :key="`${state}-${stateMode}`"
              :mode="stateMode"
              :source="stateSources[state].source"
              :initial-path="stateSources[state].path"
            />
          </div>
        </GallerySpecimen>
      </GallerySection>

      <GallerySection
        title="Address bar"
        note="One path at shrinking widths. Crumbs give up their names from the left, a hover shows each name; the file's crumb keeps its name longest, then the bar clips its left and never its right. The last frame resizes from its corner."
      >
        <GallerySpecimen v-for="entry in addressWidths" :key="entry.variant" :variant="entry.variant" wide>
          <div class="flex" :style="{ width: entry.width }">
            <FileBrowserAddressBar
              class="min-w-0 flex-1"
              mode="browse"
              :path="addressPath"
              :root="addressWorkspace.root"
              :source="addressWorkspace.source"
              leaf="file"
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="drag the corner" wide>
          <div class="flex max-w-full resize-x overflow-hidden pb-3" style="width: 30rem; min-width: 4rem">
            <FileBrowserAddressBar
              class="min-w-0 flex-1"
              mode="browse"
              :path="addressPath"
              :root="addressWorkspace.root"
              :source="addressWorkspace.source"
              leaf="file"
            />
          </div>
        </GallerySpecimen>
      </GallerySection>

      <GallerySection
        title="Narrow"
        note="At a phone width the device and nav keep the toolbar's left, the places become a menu at its right, the path takes its own row under it, Forward and the date column go, and the address bar turns its crumbs into glyphs."
      >
        <GalleryDialogFrame v-slot="{ open, close }" class="max-w-[22rem]">
          <FileBrowserDialog
            :is-open="open"
            :overlay-store="appOverlayStore"
            mode="directory"
            :source="narrowHost.source"
            :initial-path="narrowHostId === 'mac' ? '/Users/zan/Projects/a project with a very long directory name that will not fit in the address bar/src' : undefined"
            :places="narrowHost.places"
            :hosts="hosts"
            :host-id="narrowHostId"
            @update:host-id="narrowHostId = $event"
            @select="productWould(`Use ${$event} as the project's folder`)"
            @close="close"
          />
        </GalleryDialogFrame>
      </GallerySection>
    </template>
  </div>
</template>
