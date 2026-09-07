<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import type { Component } from 'vue'
import { ChevronRight, Ellipsis } from '@lucide/vue'
import { useElementSize } from '@vueuse/core'
import TextInput from '../ui/TextInput.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { normalizePath, pathSegments } from './paths'

/**
 * The path as crumbs, each a jump; a click on the bar's free space turns it into a
 * text field with the full path, the way the Windows address bar edits.
 */
const props = defineProps<{
  path: string
  /** The root crumb's label and icon: a host's name, or `/`. */
  rootLabel?: string
  rootIcon?: Component
}>()

const emit = defineEmits<{
  navigate: [path: string]
}>()

const editing = ref(false)
const draft = ref('')
const input = ref<InstanceType<typeof TextInput>>()
const bar = ref<HTMLElement>()
const { width } = useElementSize(bar)

const crumbs = computed(() => pathSegments(props.path))
/** Deep paths keep the root and the last few crumbs, fewer in a narrow bar; the middle folds into an ellipsis. */
const shown = computed(() => {
  const all = crumbs.value
  const keep = width.value > 0 && width.value < 360 ? 2 : 3
  if (all.length <= keep + 2) return all.map((crumb) => ({ ...crumb, folded: false }))
  return [
    { ...all[0]!, folded: false },
    { name: '…', path: '', folded: true },
    ...all.slice(-keep).map((crumb) => ({ ...crumb, folded: false })),
  ]
})

function startEdit() {
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
  if (next !== props.path) emit('navigate', next)
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
    class="flex h-7 min-w-0 cursor-text select-none items-center overflow-hidden rounded-md bg-surface-raised px-1 ring-1 ring-line"
    role="navigation"
    aria-label="Current path"
    @click.self="startEdit"
  >
    <template v-for="(crumb, index) in shown" :key="crumb.path || index">
      <ChevronRight v-if="index > 0" :size="ICON_PX.in24" class="shrink-0 text-fg-faint" />
      <span
        v-if="crumb.folded"
        class="flex h-5 shrink-0 items-center px-1 text-fg-subtle"
        aria-hidden="true"
      >
        <Ellipsis :size="ICON_PX.in24" />
      </span>
      <span
        v-else
        role="link"
        class="flex h-5 min-w-0 shrink items-center gap-1 rounded px-1 text-chrome transition-colors duration-200 ease-out"
        :class="[
          index === shown.length - 1 ? 'max-w-[60%] shrink-0 text-fg-emphasis' : 'text-fg-muted hover:bg-hover hover:text-fg',
          index === 0 ? 'shrink-0' : '',
        ]"
        @click="emit('navigate', crumb.path)"
      >
        <component :is="rootIcon" v-if="index === 0 && rootIcon" :size="ICON_PX.in24" class="shrink-0" />
        <span class="truncate">{{ index === 0 ? (rootLabel ?? crumb.name) : crumb.name }}</span>
      </span>
    </template>
    <span class="h-full min-w-4 flex-1" @click="startEdit" />
  </div>
</template>
