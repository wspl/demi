<script setup lang="ts" generic="T extends { id: string; label: string }">
import { computed, ref, watch } from 'vue'
import { useVirtualizer } from '@tanstack/vue-virtual'
import { SearchLine } from '@mingcute/vue/search'
import { CloseCircleFill } from '@mingcute/vue/close-circle'
import { CheckLine } from '@mingcute/vue/check'
import type { OverlayStore } from '../overlay/overlayStore'
import DropdownMenu from './DropdownMenu.vue'
import HighlightText from './HighlightText.vue'

type Placement = 'top' | 'top-start' | 'top-end' | 'bottom' | 'bottom-start' | 'bottom-end'

const props = withDefaults(defineProps<{
  overlayStore: OverlayStore
  items: T[]
  selectedId?: string
  searchable?: boolean
  searchPlaceholder?: string
  emptyText?: string
  placement?: Placement
  offset?: number
  itemHeight?: number
  matchTriggerWidth?: boolean
  anchorInset?: number
  panelClass?: string
  filterFn?: (item: T, query: string) => boolean
  noAutoFocus?: boolean
}>(), {
  searchPlaceholder: 'Search...',
  emptyText: 'No items found',
  placement: 'bottom-start',
  offset: 6,
})

const emit = defineEmits<{
  select: [id: string]
  close: []
}>()

defineSlots<{
  trigger(props: { isOpen: boolean }): void
  item(props: { item: T; query: string; isSelected: boolean }): void
}>()

const dropdownRef = ref<InstanceType<typeof DropdownMenu>>()
const filterQuery = ref('')
const focusedIndex = ref(-1)
const inputRef = ref<HTMLInputElement>()
const panelRef = ref<HTMLElement>()
const scrollRef = ref<HTMLElement>()

const filteredItems = computed(() => {
  const q = filterQuery.value.toLowerCase().trim()
  if (!q) return props.items
  const fn = props.filterFn
  if (fn) return props.items.filter(item => fn(item, q))
  return props.items.filter(item => item.label.toLowerCase().includes(q))
})

const isVirtual = computed(() => props.itemHeight != null && props.itemHeight > 0)

const virtualizer = useVirtualizer(computed(() => ({
  count: filteredItems.value.length,
  getScrollElement: () => scrollRef.value ?? null,
  estimateSize: () => props.itemHeight ?? 32,
  overscan: 5,
})))

watch(inputRef, (el) => {
  if (!el) return
  filterQuery.value = ''
  focusedIndex.value = -1
  el.focus()
})

watch(panelRef, (el) => {
  if (!el || props.searchable || props.noAutoFocus) return
  focusedIndex.value = -1
  el.focus()
})

watch(filteredItems, () => {
  focusedIndex.value = 0
})

function handleSelect(id: string) {
  emit('select', id)
  dropdownRef.value?.close()
}

function handleClear() {
  filterQuery.value = ''
  inputRef.value?.focus()
}

function handleKeydown(event: KeyboardEvent) {
  const count = filteredItems.value.length
  if (count === 0) return

  if (event.key === 'ArrowDown') {
    event.preventDefault()
    focusedIndex.value = focusedIndex.value < count - 1 ? focusedIndex.value + 1 : 0
    if (isVirtual.value) virtualizer.value.scrollToIndex(focusedIndex.value, { align: 'auto' })
    return
  }

  if (event.key === 'ArrowUp') {
    event.preventDefault()
    focusedIndex.value = focusedIndex.value > 0 ? focusedIndex.value - 1 : count - 1
    if (isVirtual.value) virtualizer.value.scrollToIndex(focusedIndex.value, { align: 'auto' })
    return
  }

  if (event.key === 'Enter' && focusedIndex.value >= 0) {
    event.preventDefault()
    handleSelect(filteredItems.value[focusedIndex.value]!.id)
  }
}

function open() {
  dropdownRef.value?.open()
}

function close() {
  dropdownRef.value?.close()
}

defineExpose({ open, close, handleKeydown })
</script>

<template>
  <DropdownMenu ref="dropdownRef" :overlay-store="overlayStore" :placement="placement" :offset="offset" :anchor-inset="anchorInset" :shift-padding="anchorInset ? 0 : undefined" @close="emit('close')">
    <template #trigger="{ isOpen }">
      <slot name="trigger" :is-open="isOpen" />
    </template>
    <template #content="{ triggerWidth }">
      <div
        ref="panelRef"
        class="flex flex-col overflow-hidden rounded-xl border border-line bg-surface text-fg shadow-xl outline-none"
        :class="panelClass ?? (!matchTriggerWidth && 'w-72')"
        :style="matchTriggerWidth ? { width: `${triggerWidth}px` } : undefined"
        tabindex="-1"
        @keydown="handleKeydown"
      >
        <div v-if="searchable" class="flex items-center gap-2 border-b border-line-subtle px-3 py-2.5 text-fg-subtle">
          <SearchLine :size="14" class="shrink-0" />
          <input
            ref="inputRef"
            v-model="filterQuery"
            type="text"
            :placeholder="searchPlaceholder"
            class="min-w-0 flex-1 bg-transparent text-[13px] text-fg-body placeholder-fg-subtle outline-none"
          />
          <span
            v-if="filterQuery"
            class="shrink-0 cursor-pointer transition-colors hover:text-fg-body"
            @click="handleClear"
          >
            <CloseCircleFill :size="14" />
          </span>
        </div>
        <div
          v-if="filteredItems.length === 0"
          class="px-3 py-4 text-center text-[13px] text-fg-subtle"
        >
          {{ emptyText }}
        </div>
        <!-- Virtual list -->
        <div v-if="isVirtual" ref="scrollRef" class="max-h-80 overflow-y-auto py-1">
          <div :style="{ height: `${virtualizer.getTotalSize()}px`, position: 'relative' }">
            <div
              v-for="vItem in virtualizer.getVirtualItems()"
              :key="String(vItem.key)"
              class="absolute inset-x-0 flex cursor-pointer items-center gap-2 px-3 transition-colors"
              :class="[
                vItem.index === focusedIndex
                  ? 'bg-active text-fg'
                  : filteredItems[vItem.index]!.id === selectedId
                    ? 'bg-overlay/5 text-fg'
                    : 'text-fg-muted hover:bg-hover hover:text-fg-body',
              ]"
              :style="{ height: `${vItem.size}px`, transform: `translateY(${vItem.start}px)` }"
              @click="handleSelect(filteredItems[vItem.index]!.id)"
            >
              <slot
                name="item"
                :item="filteredItems[vItem.index]!"
                :query="filterQuery"
                :is-selected="filteredItems[vItem.index]!.id === selectedId"
              >
                <span class="min-w-0 flex-1 truncate text-[13px]">
                  <HighlightText v-if="filterQuery" :text="filteredItems[vItem.index]!.label" :query="filterQuery" />
                  <template v-else>{{ filteredItems[vItem.index]!.label }}</template>
                </span>
              </slot>
              <span class="ml-1 w-3.5 shrink-0 text-fg-muted">
                <CheckLine v-if="filteredItems[vItem.index]!.id === selectedId" :size="14" />
              </span>
            </div>
          </div>
        </div>
        <!-- Normal list -->
        <div v-else class="max-h-80 overflow-y-auto py-1">
          <div
            v-for="(item, index) in filteredItems"
            :key="item.id"
            class="flex cursor-pointer items-center gap-2 px-3 py-1.5 transition-colors"
            :class="[
              index === focusedIndex
                ? 'bg-active text-fg'
                : item.id === selectedId
                  ? 'bg-overlay/5 text-fg'
                  : 'text-fg-muted hover:bg-hover hover:text-fg-body',
            ]"
            @click="handleSelect(item.id)"
          >
            <slot name="item" :item="item" :query="filterQuery" :is-selected="item.id === selectedId">
              <span class="min-w-0 flex-1 truncate text-[13px]">
                <HighlightText v-if="filterQuery" :text="item.label" :query="filterQuery" />
                <template v-else>{{ item.label }}</template>
              </span>
            </slot>
            <span class="ml-1 w-3.5 shrink-0 text-fg-muted">
              <CheckLine v-if="item.id === selectedId" :size="14" />
            </span>
          </div>
        </div>
      </div>
    </template>
  </DropdownMenu>
</template>
