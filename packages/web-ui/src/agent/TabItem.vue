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

const props = withDefaults(
  defineProps<{
    tab: Pick<ConversationState, 'id' | 'title'>
    isActive: boolean
    status: ConversationStatus
    isDragging?: boolean
    isDragTarget?: boolean
    isSettling?: boolean
    shift?: number
    isRenaming?: boolean
    renameValue?: string
    providerIconId?: string | null
    mark?: 'provider' | 'bot' | 'terminal'
    closable?: boolean
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
    class="relative flex h-7 max-w-[220px] shrink cursor-default items-center overflow-hidden rounded-md text-chrome select-none touch-none"
    :class="[
      isActive
        ? 'bg-surface text-fg-emphasis'
        : isDragging && isDragTarget
          ? 'bg-surface text-fg-body'
          : 'text-fg-subtle',
      !isDragging && 'group',
      !isDragging && !isActive && 'hover:bg-surface hover:text-fg-body',
      isDragging && isDragTarget && 'z-50',
    ]"
    :style="tabStyle"
    @pointerdown="emit('pointerdown', $event)"
    @pointermove="emit('pointermove', $event)"
    @pointerup="emit('pointerup')"
    @lostpointercapture="emit('lostpointercapture')"
    @contextmenu.prevent="emit('contextmenu', $event)"
  >
    <span
      v-if="uiOptions.showTabIcon"
      class="relative ml-1.5 flex shrink-0 items-center justify-center"
    >
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
      <ConversationStatusDot :status="status" />
    </span>
    <input
      v-if="isRenaming"
      ref="renameInputRef"
      :value="renameValue"
      class="w-32 truncate bg-transparent px-1.5 outline-none"
      @input="emit('update:renameValue', ($event.target as HTMLInputElement).value)"
      @keydown.enter="emit('renameSubmit')"
      @keydown.escape="emit('renameCancel')"
      @blur="emit('renameSubmit')"
      @pointerdown.stop
      @click.stop
    />
    <Tooltip
      v-else
      :content="tab.title"
      placement="bottom"
      class="w-32 truncate whitespace-nowrap px-1.5"
      >{{ tab.title }}</Tooltip
    >
    <span
      v-if="closable"
      class="pointer-events-none absolute inset-y-0 right-0 z-10 flex w-9 items-center justify-end pr-1.5 opacity-0 transition-opacity group-hover:opacity-100"
    >
      <span
        class="absolute inset-y-0 left-0 w-4"
        :class="
          isActive
            ? 'bg-linear-to-r from-surface/0 to-surface'
            : 'bg-linear-to-r from-surface-base/0 to-surface-base group-hover:from-surface/0 group-hover:to-surface'
        "
      />
      <span
        class="absolute inset-y-0 right-0 w-6"
        :class="isActive ? 'bg-surface' : 'bg-surface-base group-hover:bg-surface'"
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
  </span>
</template>
