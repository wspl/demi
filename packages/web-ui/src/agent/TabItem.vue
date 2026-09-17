<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { Bot, SquareTerminal, X } from '@lucide/vue'
import type { ConversationState } from './types'
import type { ConversationStatus } from './conversation-status'
import ProviderIcon from './providers/ProviderIcon.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import ConversationStatusDot from './ConversationStatusDot.vue'
import { useAgentUiOptions } from './ui-options'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'

const uiOptions = useAgentUiOptions()

const DRAG_TRANSITION = 'transform 120ms ease'

/**
 * One tab of a `TabStrip`. Tabs use a fixed width unless fitted to content; a faint line sits in the
 * gap after the tab and fades out while the tab or its neighbour is active or
 * hovered. The root stays a single element with no top-level comment: a
 * fragment root would keep the strip's enter and leave transitions off it.
 */
const props = withDefaults(
  defineProps<{
    tab: Pick<ConversationState, 'id' | 'title'>
    isActive: boolean
    /** The dot on the mark; none without it. */
    status?: ConversationStatus
    isDragging?: boolean
    isDragTarget?: boolean
    isSettling?: boolean
    shift?: number
    isRenaming?: boolean
    renameValue?: string
    providerIconId?: string | null
    /** The built-in mark; the `mark` slot replaces it. */
    mark?: 'provider' | 'bot' | 'terminal'
    closable?: boolean
    /** Size the tab to its icon and label instead of the default fixed width. */
    fitContent?: boolean
    /** The tooltip on the title; the title itself without it. */
    tooltip?: string
  }>(),
  {
    isDragging: false,
    isDragTarget: false,
    isSettling: false,
    shift: 0,
    isRenaming: false,
    renameValue: '',
    providerIconId: null,
    mark: 'provider',
    closable: true,
    fitContent: false,
    tooltip: undefined,
  },
)

const emit = defineEmits<{
  pointerdown: [event: PointerEvent]
  pointermove: [event: PointerEvent]
  pointerup: []
  lostpointercapture: []
  contextmenu: [event: MouseEvent]
  close: []
  renameSubmit: []
  renameCancel: []
  'update:renameValue': [value: string]
}>()

const renameInputRef = ref<HTMLInputElement | null>(null)

watch(
  () => props.isRenaming,
  (val) => {
    if (!val) {
      return
    }
    nextTick(() => {
      renameInputRef.value?.focus()
      renameInputRef.value?.select()
    })
  },
)

const tabStyle = computed(() => {
  if (!props.isDragging && !props.isSettling) {
    return undefined
  }

  return {
    transform: `translateX(${props.shift}px)`,
    transition:
      props.isSettling || !props.isDragTarget ? DRAG_TRANSITION : undefined,
  }
})
</script>

<template>
  <span
    role="tab"
    :aria-selected="isActive"
    class="relative flex h-7 shrink-0 cursor-default items-center rounded-md text-chrome select-none touch-none after:absolute after:-right-[1.5px] after:top-1/2 after:h-3.5 after:w-px after:-translate-y-1/2 after:bg-line after:transition-opacity after:duration-150 last:after:hidden hover:after:opacity-0 has-[+:hover]:after:opacity-0 has-[+[aria-selected=true]]:after:opacity-0"
    :class="[
      fitContent ? 'w-max' : 'w-40',
      isActive
        ? 'bg-(--tab-active) text-fg-emphasis after:opacity-0'
        : isDragging && isDragTarget
          ? 'bg-(--tab-active) text-fg-body'
          : 'text-fg-subtle',
      !isDragging && 'group',
      !isDragging && !isActive && 'hover:bg-(--tab-hover) hover:text-fg-body',
      isDragging && isDragTarget && 'z-50',
    ]"
    :style="tabStyle"
    @pointerdown="emit('pointerdown', $event)"
    @pointermove="emit('pointermove', $event)"
    @pointerup="emit('pointerup')"
    @lostpointercapture="emit('lostpointercapture')"
    @contextmenu.prevent="emit('contextmenu', $event)"
  >
    <!-- The tooltip covers the whole tab; the tab itself stays a plain element so the strip can transition it. -->
    <Tooltip
      tag="span"
      :content="tooltip ?? tab.title"
      placement="bottom"
      :disabled="isRenaming || isDragging"
      class="flex h-full min-w-0 flex-1 items-center"
    >
    <span
      v-if="$slots.mark || uiOptions.showTabIcon"
      class="relative ml-1.5 flex shrink-0 items-center justify-center"
    >
      <slot name="mark">
        <Bot
          v-if="mark === 'bot'"
          :size="ICON_PX.markIn28"
          class="text-fg-subtle"
        />
        <SquareTerminal
          v-else-if="mark === 'terminal'"
          :size="ICON_PX.markIn28"
          class="text-fg-subtle"
        />
        <ProviderIcon
          v-else-if="providerIconId"
          :provider-id="providerIconId"
          :size="ICON_PX.markIn28"
          class="text-fg-subtle"
        />
        <span
          v-else
          class="inline-block size-4 rounded-full bg-surface-raised"
        />
      </slot>
      <ConversationStatusDot v-if="status" :status="status" />
    </span>
    <input
      v-if="isRenaming"
      ref="renameInputRef"
      :value="renameValue"
      class="min-w-0 flex-1 truncate bg-transparent px-1.5 outline-none"
      @input="emit('update:renameValue', ($event.target as HTMLInputElement).value)"
      @keydown.enter="emit('renameSubmit')"
      @keydown.escape="emit('renameCancel')"
      @blur="emit('renameSubmit')"
      @pointerdown.stop
      @click.stop
    />
    <span
      v-else
      class="min-w-0 flex-1 truncate whitespace-nowrap px-1.5"
      ><slot name="title">{{ tab.title }}</slot></span
    >
    <span
      v-if="closable"
      class="pointer-events-none absolute inset-y-0 right-0 z-10 flex w-9 items-center justify-end overflow-hidden rounded-r-md pr-1.5 opacity-0 transition-opacity group-hover:opacity-100"
    >
      <span
        class="absolute inset-y-0 left-0 w-4"
        :class="
          isActive
            ? 'bg-linear-to-r from-transparent to-(--tab-active)'
            : 'bg-linear-to-r from-transparent to-(--tab-row) group-hover:to-(--tab-hover)'
        "
      />
      <span
        class="absolute inset-y-0 right-0 w-6"
        :class="isActive ? 'bg-(--tab-active)' : 'bg-(--tab-row) group-hover:bg-(--tab-hover)'"
      />
      <span
        role="button"
        aria-label="Close"
        class="pointer-events-auto relative z-10 flex size-5 shrink-0 items-center justify-center rounded text-fg-faint transition-colors hover:bg-hover hover:text-fg-body"
        @pointerdown.stop
        @click.stop="emit('close')"
      >
        <X :size="ICON_PX.in20" />
      </span>
    </span>
    </Tooltip>
  </span>
</template>
