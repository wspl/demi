<script setup lang="ts" generic="T extends { id: string; label: string; icon?: import('vue').Component }">
import { computed, provide, ref, watch } from 'vue'
import { useVirtualizer } from '@tanstack/vue-virtual'
import { Search, CircleX } from '@lucide/vue'
import HighlightText from './HighlightText.vue'
import MenuItem from './MenuItem.vue'
import { menuIconlessKey } from './menu-context'
import { ICON_PX } from './icon-metrics'

const props = withDefaults(defineProps<{
  items?: T[]
  selectedId?: string
  isItemDisabled?: (item: T) => boolean
  filterable?: boolean
  filterPlaceholder?: string
  emptyText?: string
  itemHeight?: number
  filterFn?: (item: T, query: string) => boolean
  autofocus?: boolean
  initialQuery?: string
  /** Drop the reserved icon column. Default for item lists that have no icons. */
  iconless?: boolean
}>(), {
  filterPlaceholder: 'Search...',
  emptyText: 'No items found',
  autofocus: true,
})

const emit = defineEmits<{
  select: [id: string]
}>()

defineSlots<{
  header(): void
  default(): void
  item(props: { item: T; query: string; isSelected: boolean }): void
}>()

const filterQuery = ref(props.initialQuery ?? '')
const focusedIndex = ref(-1)
const inputRef = ref<HTMLInputElement>()
const panelRef = ref<HTMLElement>()
const scrollRef = ref<HTMLElement>()

const listItems = computed(() => props.items ?? [])

const filteredItems = computed(() => {
  const q = filterQuery.value.toLowerCase().trim()
  if (!q) return listItems.value
  const fn = props.filterFn
  if (fn) return listItems.value.filter(item => fn(item, q))
  return listItems.value.filter(item => item.label.toLowerCase().includes(q))
})

const iconless = computed(() => {
  if (props.iconless) return true
  if (props.items == null) return false
  return props.items.every((item) => item.icon == null)
})

provide(menuIconlessKey, iconless)

const isVirtual = computed(() => props.items != null && props.itemHeight != null && props.itemHeight > 0)
const rowHeight = computed(() => props.itemHeight ?? 28)

const virtualizer = useVirtualizer(computed(() => ({
  count: filteredItems.value.length,
  getScrollElement: () => scrollRef.value ?? null,
  estimateSize: () => rowHeight.value,
  gap: 1,
  overscan: 5,
})))

watch(inputRef, (el) => {
  if (!el) return
  filterQuery.value = props.initialQuery ?? ''
  focusedIndex.value = -1
  if (props.autofocus) el.focus({ preventScroll: true })
})

watch(panelRef, (el) => {
  if (!el || props.filterable || props.items == null || !props.autofocus) return
  focusedIndex.value = -1
  el.focus({ preventScroll: true })
})

watch(filteredItems, () => {
  focusedIndex.value = 0
})

function handleSelect(id: string) {
  const item = listItems.value.find((item) => item.id === id)
  if (item && props.isItemDisabled?.(item)) return
  emit('select', id)
}

function handleClear() {
  filterQuery.value = ''
  inputRef.value?.focus({ preventScroll: true })
}

function handleKeydown(event: KeyboardEvent) {
  if (props.items == null) return
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
</script>

<template>
  <div
    ref="panelRef"
    class="overlay-panel overlay-menu min-w-[160px] rounded-lg text-fg outline-none"
    tabindex="-1"
    @keydown="handleKeydown"
  >
    <div v-if="$slots.header" class="border-b border-line p-1">
      <slot name="header" />
    </div>
    <div v-if="filterable" class="flex shrink-0 items-center gap-2 border-b border-line px-3 py-2.5 text-fg-subtle">
      <Search :size="ICON_PX.in28" class="shrink-0" />
      <input
        ref="inputRef"
        v-model="filterQuery"
        type="text"
        :placeholder="filterPlaceholder"
        class="min-w-0 flex-1 bg-transparent text-chrome text-fg-body placeholder-fg-subtle outline-none"
      />
      <span
        v-if="filterQuery"
        class="shrink-0 cursor-default transition-colors duration-200 ease-out hover:text-fg-body"
        @click="handleClear"
      >
        <CircleX :size="ICON_PX.in28" />
      </span>
    </div>
    <div
      v-if="items != null && filteredItems.length === 0"
      class="px-2 py-3 text-center text-chrome text-fg-subtle"
    >
      {{ emptyText }}
    </div>
    <div
      v-else
      ref="scrollRef"
      class="overlay-menu-scroll p-1"
    >
      <template v-if="items != null && isVirtual">
        <div :style="{ height: `${virtualizer.getTotalSize()}px`, position: 'relative' }">
          <MenuItem
            v-for="vItem in virtualizer.getVirtualItems()"
            :key="String(vItem.key)"
            class="absolute inset-x-0"
            :style="{ transform: `translateY(${vItem.start}px)` }"
            :label="filteredItems[vItem.index]!.label"
            :icon="filteredItems[vItem.index]!.icon"
            :disabled="isItemDisabled?.(filteredItems[vItem.index]!)"
            choice
            :is-selected="filteredItems[vItem.index]!.id === selectedId"
            :is-focused="vItem.index === focusedIndex"
            @select="handleSelect(filteredItems[vItem.index]!.id)"
          >
            <slot
              name="item"
              :item="filteredItems[vItem.index]!"
              :query="filterQuery"
              :is-selected="filteredItems[vItem.index]!.id === selectedId"
            >
              <span class="min-w-0 flex-1 truncate">
                <HighlightText v-if="filterQuery" :text="filteredItems[vItem.index]!.label" :query="filterQuery" />
                <template v-else>{{ filteredItems[vItem.index]!.label }}</template>
              </span>
            </slot>
          </MenuItem>
        </div>
      </template>
      <template v-else-if="items != null">
        <MenuItem
          v-for="(item, index) in filteredItems"
          :key="item.id"
          :label="item.label"
          :icon="item.icon"
          :disabled="isItemDisabled?.(item)"
          choice
          :is-selected="item.id === selectedId"
          :is-focused="index === focusedIndex"
          @select="handleSelect(item.id)"
        >
          <slot name="item" :item="item" :query="filterQuery" :is-selected="item.id === selectedId">
            <span class="min-w-0 flex-1 truncate">
              <HighlightText v-if="filterQuery" :text="item.label" :query="filterQuery" />
              <template v-else>{{ item.label }}</template>
            </span>
          </slot>
        </MenuItem>
      </template>
      <slot v-else />
    </div>
  </div>
</template>
