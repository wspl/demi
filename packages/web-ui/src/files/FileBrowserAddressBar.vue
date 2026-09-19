<script setup lang="ts">
import { computed, nextTick, provide, ref, watch } from 'vue'
import { ChevronRight } from '@lucide/vue'
import { useElementSize } from '@vueuse/core'
import { appOverlayStore } from '../overlay/appOverlay'
import Popover from '../ui/Popover.vue'
import TextInput from '../ui/TextInput.vue'
import Tooltip from '../ui/Tooltip.vue'
import { menuRootKey } from '../ui/menu-context'
import DirectoryMenu from './DirectoryMenu.vue'
import FileIcon from './FileIcon.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { fitCrumbs, type CrumbFit } from './crumb-fit'
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
 *
 * A bar too narrow for every name turns crumbs into their glyphs, one at a
 * time from the left, the name kept in a tooltip; the last crumb keeps its
 * name longest. When even the glyphs overflow, the bar keeps its right end and
 * clips its left.
 */
const props = withDefaults(
  defineProps<{
    path: string
    /** Where the root and the home are, for their glyphs; `browse` also lists through it. */
    source: Pick<FileBrowserSource, 'platform' | 'home'> & Partial<Pick<FileBrowserSource, 'list'>>
    mode?: 'navigate' | 'browse'
    /** The first crumb; the ancestors above it are not shown. */
    root?: string
    /** What the root crumb says in place of its directory's name. */
    rootName?: string
    leaf?: 'directory' | 'file'
    /** False for a bar that only shows: no text field on a click. */
    editable?: boolean
  }>(),
  { mode: 'navigate', root: undefined, rootName: undefined, leaf: 'directory', editable: true },
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
const spacer = ref<HTMLElement>()
const ruler = ref<HTMLElement>()

const crumbs = computed(() => {
  const all = pathSegments(props.path)
  if (props.root === undefined)
    return all
  const start = all.findIndex((crumb) => crumb.path === normalizePath(props.root!))
  const fromRoot = start < 0 ? all : all.slice(start)
  if (props.rootName === undefined || start < 0)
    return fromRoot
  return [{ ...fromRoot[0]!, name: props.rootName }, ...fromRoot.slice(1)]
})
/** What a crumb is laid out as, shown or measured: the classes that decide its width. */
const CRUMB_LAYOUT = 'flex h-5 shrink-0 items-center gap-1 rounded px-1 text-chrome'

const fit = ref<CrumbFit>({ collapsed: 0, clipped: false })

function width(el: Element | null): number {
  return el ? el.getBoundingClientRect().width : 0
}

/** Reads every crumb's width from the ruler, and fits them to the bar's room less the spacer's minimum. */
function measure(): void {
  if (!bar.value || !spacer.value || !ruler.value)
    return
  // Each crumb's ruler entry holds it with its name, then as its glyph alone.
  const widths = [...ruler.value.querySelectorAll('[data-crumb]')].map((entry) => ({
    full: width(entry.firstElementChild),
    icon: width(entry.lastElementChild),
  }))
  const style = getComputedStyle(bar.value)
  const available = bar.value.clientWidth -
    Number.parseFloat(style.paddingLeft) -
    Number.parseFloat(style.paddingRight) -
    Number.parseFloat(getComputedStyle(spacer.value).minWidth)
  fit.value = fitCrumbs(widths, width(ruler.value.querySelector('[data-separator]')), available)
}

// The bar's width, and the ruler's for names that change width as the path or a font does.
const { width: barWidth } = useElementSize(bar)
const { width: rulerWidth } = useElementSize(ruler)
watch([barWidth, rulerWidth, crumbs], measure, { flush: 'post', immediate: true })

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
    class="relative flex h-7 min-w-0 cursor-default select-none items-center overflow-hidden rounded-md bg-surface-raised px-1 ring-1 ring-line"
    role="navigation"
    aria-label="Current path"
    @click.self="startEdit"
  >
    <!-- Clipped, the row ends at its right edge, so what overflows is the left. -->
    <div class="flex min-w-0 items-center overflow-hidden" :class="fit.clipped ? 'justify-end' : ''">
      <template v-for="(crumb, index) in crumbs" :key="crumb.path">
        <ChevronRight
          v-if="index > 0"
          :size="ICON_PX.in24"
          class="shrink-0 text-fg-faint"
        />
        <Tooltip :content="crumb.name" :disabled="index >= fit.collapsed" class="shrink-0">
          <span
            :ref="(el) => bindCrumb(crumb.path, el)"
            :role="mode === 'browse' ? 'button' : 'link'"
            :aria-label="index < fit.collapsed ? crumb.name : undefined"
            :aria-expanded="mode === 'browse' ? menuCrumb?.path === crumb.path : undefined"
            class="transition-colors duration-200 ease-out"
            :class="[
              CRUMB_LAYOUT,
              index === crumbs.length - 1 ? 'text-fg-emphasis' : 'text-fg-muted',
              index === crumbs.length - 1 && mode === 'navigate' ? '' : 'hover:bg-hover hover:text-fg',
              menuCrumb?.path === crumb.path ? 'bg-hover text-fg' : '',
            ]"
            @click="onCrumbClick(crumb.path)"
          >
            <!-- The root and the home wear their landmark glyphs; every crumb has the same shape. -->
            <FileIcon
              :name="crumb.name"
              :is-directory="!(leaf === 'file' && index === crumbs.length - 1)"
              :icon="landmarkIcon(crumb.path, source)"
            />
            <span v-if="index >= fit.collapsed" class="whitespace-nowrap">{{ crumb.name }}</span>
          </span>
        </Tooltip>
      </template>
    </div>
    <span ref="spacer" class="h-full min-w-4 flex-1" @click="startEdit" />
    <!-- Every crumb both ways and a separator, out of sight, so the bar knows what fits. -->
    <div ref="ruler" class="pointer-events-none invisible absolute left-0 top-0 flex w-max items-center" aria-hidden="true">
      <span data-separator class="flex shrink-0"><ChevronRight :size="ICON_PX.in24" /></span>
      <span v-for="(crumb, index) in crumbs" :key="crumb.path" data-crumb class="flex">
        <span :class="CRUMB_LAYOUT">
          <FileIcon :name="crumb.name" :is-directory="!(leaf === 'file' && index === crumbs.length - 1)" />
          <span class="whitespace-nowrap">{{ crumb.name }}</span>
        </span>
        <span :class="CRUMB_LAYOUT">
          <FileIcon :name="crumb.name" :is-directory="!(leaf === 'file' && index === crumbs.length - 1)" />
        </span>
      </span>
    </div>
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
