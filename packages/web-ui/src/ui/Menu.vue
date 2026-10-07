<script setup lang="ts" generic="T extends import('./menu-context').MenuListItem">
import { computed, inject, onBeforeUnmount, provide, ref, shallowRef, watch } from 'vue'
import { useVirtualizer } from '@tanstack/vue-virtual'
import { Search, CircleX } from '@lucide/vue'
import { useTypeSelect } from '../composables/useTypeSelect'
import HighlightText from './HighlightText.vue'
import MenuItem from './MenuItem.vue'
import ScrollArea from './ScrollArea.vue'
import TypeSelectHint from './TypeSelectHint.vue'
import {
  createSubmenuController,
  menuIconlessKey,
  menuSlotKeyboardKey,
  menuSubmenuKey,
  type MenuSlotRow
} from './menu-context'
import { provideLayerElevation } from '../overlay/layerElevation'
import { ICON_PX } from './icon-metrics'
import type { HeadlineText, PlaceholderText } from './ui-text'

/**
 * A menu: `items` it lays out itself, filterable and virtual when asked, or
 * MenuItems in its slot. It takes the keyboard when it opens, unless it is a
 * submenu or `autofocus` is off. Without a filter field, typing a name moves
 * to the first row that starts with it (`useTypeSelect`), and Enter chooses
 * the row the keys moved to.
 */
const props = withDefaults(defineProps<{
  items?: T[]
  selectedId?: string
  isItemDisabled?: (item: T) => boolean
  filterable?: boolean
  filterPlaceholder?: PlaceholderText
  emptyText?: HeadlineText
  itemHeight?: number
  filterFn?: (item: T, query: string) => boolean
  autofocus?: boolean
  initialQuery?: string
  /** Drop the reserved icon column. Default for item lists that have no icons. */
  iconless?: boolean
}>(), {
  filterPlaceholder: 'Search…',
  emptyText: 'No Items Found',
  autofocus: true,
  iconless: undefined,
})

const emit = defineEmits<{
  select: [id: string]
}>()

defineSlots<{
  header(): void
  default(): void
  item(props: {
    item: T;
    query: string;
    isSelected: boolean
  }): void
}>()

// A menu in another's submenu leaves the keyboard with the outermost one.
const nested = inject(menuSubmenuKey, null) !== null

const filterQuery = ref(props.initialQuery ?? '')
const focusedIndex = ref(-1)
const inputRef = ref<HTMLInputElement>()
const panelRef = ref<HTMLElement>()
const scrollArea = ref<InstanceType<typeof ScrollArea>>()
const scrollRef = computed(() => scrollArea.value?.el)

const listItems = computed(() => props.items ?? [])

const filteredItems = computed(() => {
  const q = filterQuery.value.toLowerCase().trim()
  if (!q)
    return listItems.value
  const fn = props.filterFn
  if (fn)
    return listItems.value.filter(item => fn(item, q))
  return listItems.value.filter(item => item.label.toLowerCase().includes(q))
})

const iconless = computed(() => {
  if (props.iconless != null)
    return props.iconless
  if (props.items == null)
    return false
  return props.items.every((item) => item.icon == null && item.indicator == null)
})

provide(menuIconlessKey, iconless)

const elevation = provideLayerElevation()

// The MenuItems of a menu that lays out its slot, which its keyboard reaches through.
const slotRows = shallowRef<MenuSlotRow[]>([])
const focusedSlotRow = shallowRef<MenuSlotRow | null>(null)

/** The slot's rows, top to bottom as they show. */
function orderedSlotRows(): MenuSlotRow[] {
  return slotRows.value
    .flatMap((row) => {
      const el = row.el()
      return el ? [{ row, el }] : []
    })
    .sort((a, b) => (a.el.compareDocumentPosition(b.el) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1))
    .map(({ row }) => row)
}

/** Moves the keyboard to the row at `index` of the rows shown, and brings it into view. */
function focusRow(index: number): void {
  if (props.items == null) {
    const row = orderedSlotRows()[index] ?? null
    focusedSlotRow.value = row
    row?.el()?.scrollIntoView({ block: 'nearest' })
    return
  }
  focusedIndex.value = index
  if (isVirtual.value)
    virtualizer.value.scrollToIndex(index, { align: 'auto' })
  else
    scrollRef.value?.querySelectorAll('[data-menu-item]')[index]?.scrollIntoView({ block: 'nearest' })
}

const typeSelect = useTypeSelect({
  names: () => props.items == null
    ? orderedSlotRows().map((row) => row.label() ?? '')
    : filteredItems.value.map((item) => item.label),
  select: focusRow,
})

/** The typed prefix to mark on the row at `index`: only the focused one shows it. */
function typedPrefix(index: number, label: string): readonly number[] | undefined {
  return index === focusedIndex.value ? (typeSelect.prefix(label) ?? undefined) : undefined
}

provide(menuSlotKeyboardKey, {
  register(row) {
    slotRows.value = [...slotRows.value, row]
    return () => {
      slotRows.value = slotRows.value.filter((entry) => entry !== row)
      if (focusedSlotRow.value === row)
        focusedSlotRow.value = null
    }
  },
  focused: focusedSlotRow,
  highlight: typeSelect.prefix,
})

const submenus = createSubmenuController()
provide(menuSubmenuKey, submenus)
onBeforeUnmount(submenus.dispose)

const isVirtual = computed(
  () => props.items != null && props.itemHeight != null && props.itemHeight > 0
)
const rowHeight = computed(() => props.itemHeight ?? 28)

const virtualizer = useVirtualizer(computed(() => ({
  count: filteredItems.value.length,
  getScrollElement: () => scrollRef.value ?? null,
  estimateSize: () => rowHeight.value,
  gap: 1,
  overscan: 5,
})))

watch(inputRef, (el) => {
  if (!el)
    return
  filterQuery.value = props.initialQuery ?? ''
  focusedIndex.value = -1
  if (props.autofocus)
    el.focus({ preventScroll: true })
})

watch(panelRef, (el) => {
  if (!el || props.filterable || !props.autofocus)
    return
  // A slot's submenu leaves the keys with its parent; a slot that focused a field of its own keeps it.
  if (props.items == null && (nested || el.contains(document.activeElement)))
    return
  focusedIndex.value = -1
  el.focus({ preventScroll: true })
})

watch(filteredItems, () => {
  focusedIndex.value = 0
})

function handleSelect(id: string) {
  const item = listItems.value.find((item) => item.id === id)
  if (item && props.isItemDisabled?.(item))
    return
  emit('select', id)
}

function handleClear() {
  filterQuery.value = ''
  inputRef.value?.focus({ preventScroll: true })
}

function handleKeydown(event: KeyboardEvent) {
  // Type-select reads the keys the panel itself gets; a field in the header keeps its own.
  if (!props.filterable && event.target === event.currentTarget && typeSelect.keydown(event)) {
    event.preventDefault()
    return
  }
  if (props.items == null) {
    if (event.key === 'Enter' && focusedSlotRow.value) {
      event.preventDefault()
      // As a click on it: the row does what a click does, closing its menu or opening its submenu.
      focusedSlotRow.value.el()?.click()
    }
    return
  }
  const count = filteredItems.value.length
  if (count === 0)
    return

  if (event.key === 'ArrowDown') {
    event.preventDefault()
    focusRow(focusedIndex.value < count - 1 ? focusedIndex.value + 1 : 0)
    return
  }

  if (event.key === 'ArrowUp') {
    event.preventDefault()
    focusRow(focusedIndex.value > 0 ? focusedIndex.value - 1 : count - 1)
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
    role="menu"
    class="overlay-panel overlay-menu rounded-lg text-fg outline-none"
    :class="filterable ? 'min-w-48' : 'min-w-40'"
    :style="{ '--elevation': elevation }"
    tabindex="-1"
    @keydown="handleKeydown"
  >
    <div v-if="$slots.header" class="border-b border-line p-1">
      <slot name="header" />
    </div>
    <div
      v-if="filterable"
      class="flex shrink-0 items-center gap-2 border-b border-line px-3 py-2.5 text-fg-subtle"
    >
      <Search :size="ICON_PX.in28" class="shrink-0" />
      <input
        ref="inputRef"
        v-model="filterQuery"
        type="text"
        :placeholder="filterPlaceholder"
        class="w-0 min-w-0 flex-1 bg-transparent text-chrome text-fg-body placeholder-fg-subtle outline-none"
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
    <ScrollArea
      v-else
      ref="scrollArea"
      class="flex-auto"
      viewport-class="overlay-menu-scroll p-1"
    >
      <template v-if="items != null && isVirtual">
        <div
          :style="{ height: `${virtualizer.getTotalSize()}px`, position: 'relative' }"
        >
          <MenuItem
            v-for="vItem in virtualizer.getVirtualItems()"
            :key="String(vItem.key)"
            class="absolute inset-x-0"
            :style="{ transform: `translateY(${vItem.start}px)` }"
            :label="filteredItems[vItem.index]!.label"
            :icon="filteredItems[vItem.index]!.icon"
            :note="filteredItems[vItem.index]!.note"
            :indicator="filteredItems[vItem.index]!.indicator"
            :indicator-label="filteredItems[vItem.index]!.indicatorLabel"
            :disabled-reason="filteredItems[vItem.index]!.disabledReason"
            :value="filteredItems[vItem.index]!.value"
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
                <HighlightText
                  :text="filteredItems[vItem.index]!.label"
                  :query="filterQuery"
                  :indexes="typedPrefix(vItem.index, filteredItems[vItem.index]!.label)"
                />
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
          :note="item.note"
          :indicator="item.indicator"
          :indicator-label="item.indicatorLabel"
          :disabled-reason="item.disabledReason"
          :value="item.value"
          :disabled="isItemDisabled?.(item)"
          choice
          :is-selected="item.id === selectedId"
          :is-focused="index === focusedIndex"
          @select="handleSelect(item.id)"
        >
          <slot
            name="item"
            :item="item"
            :query="filterQuery"
            :is-selected="item.id === selectedId"
          >
            <span class="min-w-0 flex-1 truncate">
              <HighlightText :text="item.label" :query="filterQuery" :indexes="typedPrefix(index, item.label)" />
            </span>
          </slot>
        </MenuItem>
      </template>
      <slot v-else />
    </ScrollArea>
    <TypeSelectHint v-if="!filterable" :query="typeSelect.query.value" :matched="typeSelect.matched.value" />
  </div>
</template>
