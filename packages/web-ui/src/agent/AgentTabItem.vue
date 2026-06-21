<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { CloseLine } from '@mingcute/vue/close'
import type { ConversationState } from './types'
import type { ConversationStatus } from './conversation-status'
import ProviderIcon from './providers/ProviderIcon.vue'
import Tooltip from '@demi/web-ui/ui/Tooltip.vue'
import ConversationStatusDot from './ConversationStatusDot.vue'

const TAB_TRANSITION = 'max-width 200ms ease-out, min-width 200ms ease-out, padding 200ms ease-out, opacity 150ms ease, background-color 150ms ease, color 150ms ease'
const DRAG_TRANSITION = 'transform 120ms ease'

const props = defineProps<{
  tab: ConversationState
  isActive: boolean
  status: ConversationStatus
  isClosing: boolean
  isEntering: boolean
  isDragging: boolean
  isDragTarget: boolean
  isSettling: boolean
  shift: number
  isRenaming: boolean
  renameValue: string
  providerIconId: string | null
}>()

const emit = defineEmits<{
  pointerdown: [event: PointerEvent]
  pointermove: [event: PointerEvent]
  pointerup: []
  lostpointercapture: []
  transitionend: [event: TransitionEvent]
  contextmenu: [event: MouseEvent]
  close: []
  renameSubmit: []
  renameCancel: []
  'update:renameValue': [value: string]
}>()

const renameInputRef = ref<HTMLInputElement | null>(null)

watch(() => props.isRenaming, (val) => {
  if (!val) return
  nextTick(() => {
    renameInputRef.value?.focus()
    renameInputRef.value?.select()
  })
})

const tabStyle = computed(() => {
  const isCollapsed = props.isClosing || props.isEntering

  if (props.isDragging || props.isSettling) {
    return {
      maxWidth: isCollapsed ? '0px' : '220px',
      minWidth: isCollapsed ? '0px' : undefined,
      paddingLeft: isCollapsed ? '0px' : undefined,
      paddingRight: isCollapsed ? '0px' : undefined,
      opacity: isCollapsed ? '0' : undefined,
      transform: `translateX(${props.shift}px)`,
      transition: (props.isSettling || !props.isDragTarget) ? DRAG_TRANSITION : undefined,
    }
  }

  return isCollapsed
    ? { maxWidth: '0px', minWidth: '0px', paddingLeft: '0px', paddingRight: '0px', opacity: '0', transition: TAB_TRANSITION }
    : { maxWidth: '220px', transition: TAB_TRANSITION }
})
</script>

<template>
  <span
    class="relative flex shrink cursor-pointer items-center overflow-hidden rounded-lg text-[13px] select-none touch-none"
    :class="[
      isActive
        ? 'bg-surface text-fg-emphasis'
        : isDragging && isDragTarget
          ? 'bg-surface text-fg-body'
          : 'text-fg-subtle',
      !isDragging && 'group',
      !isDragging && !isActive && 'hover:bg-surface hover:text-fg-body',
      isDragging && isDragTarget && 'z-50',
      isClosing && 'pointer-events-none',
    ]"
    :style="tabStyle"
    @pointerdown="emit('pointerdown', $event)"
    @pointermove="emit('pointermove', $event)"
    @pointerup="emit('pointerup')"
    @lostpointercapture="emit('lostpointercapture')"
    @transitionend="emit('transitionend', $event)"
    @contextmenu.prevent="emit('contextmenu', $event)"
  >
    <span class="relative ml-1.5 flex shrink-0 items-center justify-center">
      <ProviderIcon
        v-if="providerIconId"
        :provider-id="providerIconId"
        :size="18"
        class="text-fg-subtle"
      />
      <span
        v-else
        class="inline-block size-4 rounded-full bg-surface-raised"
      />
      <ConversationStatusDot
        :status="status"
      />
    </span>
    <input
      v-if="isRenaming"
      ref="renameInputRef"
      :value="renameValue"
      class="w-32 truncate bg-transparent pl-1.5 pr-1.5 py-1.5 outline-none"
      @input="emit('update:renameValue', ($event.target as HTMLInputElement).value)"
      @keydown.enter="emit('renameSubmit')"
      @keydown.escape="emit('renameCancel')"
      @blur="emit('renameSubmit')"
      @pointerdown.stop
      @click.stop
    />
    <Tooltip v-else :content="tab.title" placement="bottom" class="w-32 truncate whitespace-nowrap pl-1.5 pr-1.5 py-1.5">{{ tab.title }}</Tooltip>
    <span
      class="pointer-events-none absolute inset-y-0 right-0 z-10 flex w-9 items-center justify-end pr-1.5 opacity-0 transition-opacity group-hover:opacity-100"
    >
      <span
        class="absolute inset-y-0 left-0 w-4"
        :class="isActive
          ? 'bg-linear-to-r from-surface/0 to-surface'
          : 'bg-linear-to-r from-surface-base/0 to-surface-base group-hover:from-surface/0 group-hover:to-surface'"
      />
      <span
        class="absolute inset-y-0 right-0 w-6"
        :class="isActive ? 'bg-surface' : 'bg-surface-base group-hover:bg-surface'"
      />
      <span
        class="pointer-events-auto relative z-10 flex size-5 shrink-0 items-center justify-center rounded text-fg-faint transition-colors hover:bg-hover hover:text-fg-body"
        @pointerdown.stop
        @click.stop="emit('close')"
      >
        <CloseLine :size="12" />
      </span>
    </span>
  </span>
</template>
