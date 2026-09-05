<script setup lang="ts">
import { computed, ref } from 'vue'
import { ArrowLeft, FileText, Folder, X } from '@lucide/vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { useResources } from '../prototype/resources'

const props = defineProps<{ deviceId: string; initialPath: string; locked: boolean }>()
const emit = defineEmits<{ close: []; select: [path: string] }>()
const resources = useResources()
const path = ref(props.initialPath.replace(/\/$/, '') || '/')
const search = ref('')
const preview = ref<string | null>(null)
const device = computed(() => resources.devices.find((item) => item.id === props.deviceId))
const roots = new Set([
  path.value,
  ...resources.projects
    .filter((project) => project.deviceId === props.deviceId)
    .map((project) => project.path),
])
// These file contents are illustrative fixtures, never reads from the user's disk.
const files = new Map<string, string>()
for (const root of roots) {
  const base = root === '/' ? '' : root
  files.set(`${base}/README.md`, '# Workspace\n\nProject source and documentation.\n')
  files.set(`${base}/src/index.ts`, 'export const greeting = "Hello, Demi"\n')
  files.set(`${base}/docs/notes.md`, '# Notes\n\nA place for the next idea.\n')
}
const directories = new Set(['/'])
for (const file of files.keys()) {
  const parts = file.split('/').filter(Boolean)
  for (let i = 1; i < parts.length; i++) directories.add(`/${parts.slice(0, i).join('/')}`)
}
const entries = computed(() => {
  const prefix = path.value === '/' ? '/' : `${path.value}/`
  return [...directories, ...files.keys()]
    .filter(
      (entry) =>
        entry.startsWith(prefix) &&
        entry !== path.value &&
        !entry.slice(prefix.length).includes('/'),
    )
    .map((entry) => ({
      path: entry,
      name: entry.slice(prefix.length),
      directory: directories.has(entry),
    }))
    .filter((entry) => entry.name.toLowerCase().includes(search.value.toLowerCase()))
    .sort((a, b) => Number(b.directory) - Number(a.directory) || a.name.localeCompare(b.name))
})
function navigate(next: string) {
  path.value = next
  search.value = ''
  preview.value = null
}
function up() {
  navigate(path.value.slice(0, path.value.lastIndexOf('/')) || '/')
}
function open(entry: { path: string; directory: boolean }) {
  if (entry.directory) navigate(entry.path)
  else preview.value = entry.path
}
</script>

<template>
  <Dialog
    :is-open="true"
    :overlay-store="appOverlayStore"
    label="Workspace files"
    size="lg"
    @close="emit('close')"
  >
    <header class="flex select-none items-center justify-between border-b border-line px-4 py-3">
      <div>
        <h2 class="text-chrome font-medium">Workspace files · {{ device?.name ?? 'Cloud' }}</h2>
      </div>
      <IconButton
        :icon="X"
        variant="ghost"
        aria-label="Close file browser"
        @click="emit('close')"
      />
    </header>
    <div class="flex items-center gap-2 p-3">
      <IconButton
        :icon="ArrowLeft"
        variant="ghost"
        aria-label="Parent directory"
        :disabled="path === '/'"
        @click="up"
      />
      <TextInput :model-value="path" readonly aria-label="Current directory" />
    </div>
    <div class="px-3 pb-2">
      <TextInput v-model="search" aria-label="Filter files" placeholder="Filter files…" />
    </div>
    <div class="grid min-h-64 sm:grid-cols-2">
      <Menu class="max-h-80 overflow-y-auto rounded-none" :autofocus="false">
        <MenuItem
          v-for="entry in entries"
          :key="entry.path"
          :icon="entry.directory ? Folder : FileText"
          :label="entry.name"
          @select="open(entry)"
        />
        <p v-if="!entries.length" class="p-3 text-chrome text-fg-subtle">
          No matching files or folders.
        </p>
      </Menu>
      <div class="min-w-0 border-t border-line bg-surface-base p-4 sm:border-l sm:border-t-0">
        <template v-if="preview">
          <p class="mb-3 break-all text-[11px] text-fg-subtle">{{ preview }}</p>
          <pre class="whitespace-pre-wrap break-words text-[12px] text-fg-body">{{
            files.get(preview)
          }}</pre>
        </template>
      </div>
    </div>
    <footer class="flex items-center justify-end gap-3 border-t border-line p-3">
      <Button
        :disabled="locked || (deviceId !== 'cloud' && !device?.online)"
        @click="emit('select', path)"
      >
        Use this folder
      </Button>
    </footer>
  </Dialog>
</template>
