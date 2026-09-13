<script setup lang="ts">
import { computed, nextTick, provide, ref, watch } from 'vue'
import { ChevronRight, Ellipsis } from '@lucide/vue'
import { useElementSize } from '@vueuse/core'
import { appOverlayStore } from '../overlay/appOverlay'
import Popover from '../ui/Popover.vue'
import TextInput from '../ui/TextInput.vue'
import { menuRootKey } from '../ui/menu-context'
import DirectoryMenu from './DirectoryMenu.vue'
import FileIcon from './FileIcon.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { landmarkIcon } from './file-icons'
import { normalizePath, parentPath, pathSegments } from './paths'
import type { FileBrowserSource } from './types'

/**
 * The path as crumbs, each its folder glyph, in one of two modes. `navigate`
 * (a file dialog): a crumb jumps there, and a click on the bar's free space
 * turns it into a text field with the full path, the way the Windows address
 * bar edits. `browse` (a file view): a crumb opens a menu of what lies beside
 * it, directories unfolding into their own, so another file is a pick away;
 * nothing edits. A bar with a `root` starts its crumbs there, the way a file
 * view shows a path inside its workspace; `leaf` says what the last crumb is,
 * so a file shows its file glyph.
 */
const props = withDefaults(
  defineProps<{
    path: string
    /** Where the root and the home are, for their glyphs; `browse` also lists through it. */
    source: Pick<FileBrowserSource, 'platform' | 'home'> & Partial<Pick<FileBrowserSource, 'list'>>
    mode?: 'navigate' | 'browse'
    /** The first crumb; the ancestors above it are not shown. */
    root?: string
    leaf?: 'directory' | 'file'
    /** False for a bar that only shows: no text field on a click. */
    editable?: boolean
  }>(),
  { mode: 'navigate', root: undefined, leaf: 'directory', editable: true },
)

const emit = defineEmits<{
  navigate: [path: string]
  /** `browse`: a file picked from a crumb's menu. */
  open: [path: string]
}>()

// `browse`: the crumb whose menu is open, and what that menu lists: the root
// crumb its own entries, any other the entries beside it.
const crumbEls = new Map<string, HTMLElement>()
const menuCrumb = ref<{ path: string; el: HTMLElement } | null>(null)
const menuDirectory = computed(() => {
  const crumb = menuCrumb.value
  if (!crumb) {
    return null
  }
  return crumb.path === crumbs.value[0]?.path ? crumb.path : parentPath(crumb.path)
})

function bindCrumb(path: string, el: unknown): void {
  if (el instanceof HTMLElement) {
    crumbEls.set(path, el)
  } else {
    crumbEls.delete(path)
  }
}

function onCrumbClick(path: string): void {
  if (props.mode === 'navigate') {
    emit('navigate', path)
    return
  }
  const el = crumbEls.get(path)
  if (!el) {
    return
  }
  menuCrumb.value = menuCrumb.value?.path === path ? null : { path, el }
}

function closeMenu(): void {
  menuCrumb.value = null
}

function pick(path: string): void {
  closeMenu()
  emit('open', path)
}

provide(menuRootKey, { dismiss: closeMenu })

const editing = ref(false)
const draft = ref('')
const input = ref<InstanceType<typeof TextInput>>()
const bar = ref<HTMLElement>()
const { width } = useElementSize(bar)

const crumbs = computed(() => {
  const all = pathSegments(props.path)
  if (props.root === undefined)
    return all
  const start = all.findIndex((crumb) => crumb.path === normalizePath(props.root!))
  return start < 0 ? all : all.slice(start)
})
/** Deep paths keep the root and the last few crumbs, fewer in a narrow bar; the middle folds into an ellipsis. */
const shown = computed(() => {
  const all = crumbs.value
  const keep = width.value > 0 && width.value < 360 ? 2 : 3
  if (all.length <= keep + 2)
    return all.map((crumb) => ({ ...crumb, folded: false }))
  return [
    { ...all[0]!, folded: false },
    { name: '…', path: '', folded: true },
    ...all.slice(-keep).map((crumb) => ({ ...crumb, folded: false })),
  ]
})

function startEdit() {
  if (!props.editable)
    return
  draft.value = props.path
  editing.value = true
  nextTick(() => {
    input.value?.focus()
    input.value?.select()
  })
}

function commit() {
  editing.value = false
  const next = normalizePath(draft.value)
  if (next !== props.path)
    emit('navigate', next)
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Enter') {
    event.preventDefault()
    commit()
  } else if (event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    editing.value = false
  }
}

watch(() => props.path, () => {
  editing.value = false
  closeMenu()
})
</script>

<template>
  <TextInput
    v-if="editing"
    ref="input"
    v-model="draft"
    aria-label="Path"
    spellcheck="false"
    @keydown="onKeydown"
    @blur="editing = false"
  />
  <div
    v-else
    ref="bar"
    class="flex h-7 min-w-0 cursor-default select-none items-center overflow-hidden rounded-md bg-surface-raised px-1 ring-1 ring-line"
    role="navigation"
    aria-label="Current path"
    @click.self="startEdit"
  >
    <template v-for="(crumb, index) in shown" :key="crumb.path || index">
      <ChevronRight
        v-if="index > 0"
        :size="ICON_PX.in24"
        class="shrink-0 text-fg-faint"
      />
      <span
        v-if="crumb.folded"
        class="flex h-5 shrink-0 items-center px-1 text-fg-subtle"
        aria-hidden="true"
      >
        <Ellipsis :size="ICON_PX.in24" />
      </span>
      <span
        v-else
        :ref="(el) => bindCrumb(crumb.path, el)"
        :role="mode === 'browse' ? 'button' : 'link'"
        :aria-expanded="mode === 'browse' ? menuCrumb?.path === crumb.path : undefined"
        class="flex h-5 min-w-0 shrink items-center gap-1 rounded px-1 text-chrome transition-colors duration-200 ease-out"
        :class="[
          index === shown.length - 1 ? 'max-w-[60%] shrink-0 text-fg-emphasis' : 'text-fg-muted',
          index === shown.length - 1 && mode === 'navigate' ? '' : 'hover:bg-hover hover:text-fg',
          menuCrumb?.path === crumb.path ? 'bg-hover text-fg' : '',
          index === 0 ? 'shrink-0' : '',
        ]"
        @click="onCrumbClick(crumb.path)"
      >
        <!-- The root and the home wear their landmark glyphs; every crumb has the same shape. -->
        <FileIcon
          :name="crumb.name"
          :is-directory="!(leaf === 'file' && index === shown.length - 1)"
          :icon="landmarkIcon(crumb.path, source)"
        />
        <span class="truncate">{{ crumb.name }}</span>
      </span>
    </template>
    <span class="h-full min-w-4 flex-1" @click="startEdit" />
    <!-- Inside the bar, so the bar stays the component's one root and keeps the host's classes. -->
    <Popover
      v-if="mode === 'browse' && source.list"
      :overlay-store="appOverlayStore"
      :is-open="menuCrumb !== null"
      :anchor-el="menuCrumb?.el ?? null"
      :ignore-els="menuCrumb ? [menuCrumb.el] : []"
      placement="bottom-start"
      :offset="4"
      @close="closeMenu"
    >
      <DirectoryMenu
        v-if="menuDirectory !== null"
        :source="{ list: source.list }"
        :path="menuDirectory"
        :current="menuCrumb?.path"
        @pick="pick"
      />
    </Popover>
  </div>
</template>
