<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import { useFloating, offset as offsetMiddleware, flip, shift, autoUpdate } from '@floating-ui/vue'
import type { TokenUsage } from '@demi/core'
import { t } from '@demi/web-ui/infra/i18n'

const props = defineProps<{
  usage: TokenUsage
  contextWindow: number
}>()

function formatTokens(tokens: number): string {
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(1)}K`
  return String(tokens)
}

const contextWindow = computed(() => props.contextWindow)
const totalUsageTokens = computed(() =>
  props.usage.cacheReadTokens
  + props.usage.inputTokens
  + props.usage.outputTokens
  + props.usage.cacheWriteTokens,
)

const barSegments = computed(() => {
  const limit = contextWindow.value
  if (limit <= 0) {
    return [{ key: 'remaining', width: 100, className: 'bg-overlay/8' }]
  }

  const cacheReadWidth = Math.min((props.usage.cacheReadTokens / limit) * 100, 100)
  const inputWidth = Math.min((props.usage.inputTokens / limit) * 100, 100 - cacheReadWidth)
  const outputWidth = Math.min((props.usage.outputTokens / limit) * 100, 100 - cacheReadWidth - inputWidth)
  const remainingWidth = Math.max(100 - cacheReadWidth - inputWidth - outputWidth, 0)

  return [
    { key: 'cache-read', width: cacheReadWidth, className: 'bg-emerald-400' },
    { key: 'input', width: inputWidth, className: 'bg-red-400' },
    { key: 'output', width: outputWidth, className: 'bg-overlay/8' },
    { key: 'remaining', width: remainingWidth, className: 'bg-overlay/8' },
  ].filter((segment) => segment.width > 0)
})

const statItems = computed(() => [
  { label: t('agent.stats.contextWindow'), value: contextWindow.value > 0 ? formatTokens(contextWindow.value) : t('agent.stats.unavailable'), valueClass: 'text-fg' },
  { label: t('agent.stats.cacheRead'), value: formatTokens(props.usage.cacheReadTokens), valueClass: 'text-on-success' },
  { label: t('agent.stats.input'), value: formatTokens(props.usage.inputTokens), valueClass: 'text-on-danger' },
  { label: t('agent.stats.output'), value: formatTokens(props.usage.outputTokens), valueClass: 'text-fg-body' },
  { label: t('agent.stats.cacheWrite'), value: formatTokens(props.usage.cacheWriteTokens), valueClass: 'text-on-violet' },
  { label: t('agent.stats.totalUsage'), value: formatTokens(totalUsageTokens.value), valueClass: 'text-fg' },
])

const triggerRef = ref<HTMLElement | null>(null)
const floatingRef = ref<HTMLElement | null>(null)
const isPopoverOpen = ref(false)

const { floatingStyles } = useFloating(triggerRef, floatingRef, {
  placement: 'top-end',
  middleware: [offsetMiddleware(10), flip(), shift({ padding: 8 })],
  whileElementsMounted: autoUpdate,
  transform: false,
})

let closeTimer: ReturnType<typeof setTimeout> | null = null

function clearCloseTimer() {
  if (!closeTimer) return
  clearTimeout(closeTimer)
  closeTimer = null
}

function openPopover() {
  clearCloseTimer()
  isPopoverOpen.value = true
}

function scheduleClosePopover() {
  clearCloseTimer()
  closeTimer = setTimeout(() => {
    closeTimer = null
    isPopoverOpen.value = false
  }, 80)
}

onBeforeUnmount(() => {
  clearCloseTimer()
})
</script>

<template>
  <div class="relative h-0">
    <div class="absolute right-8 top-0 flex -translate-y-1/2 justify-end">
      <div
        ref="triggerRef"
        class="flex h-5 w-12 cursor-default items-center justify-end"
        @mouseenter="openPopover"
        @mouseleave="scheduleClosePopover"
      >
        <div class="flex h-0.5 w-8 overflow-hidden rounded-full bg-overlay/8">
          <div
            v-for="segment in barSegments"
            :key="segment.key"
            class="h-full"
            :class="segment.className"
            :style="{ width: `${segment.width}%` }"
          />
        </div>
      </div>
    </div>
  </div>

  <Teleport to="body">
    <Transition
      enter-active-class="transition-[opacity,transform] duration-120 ease-out"
      leave-active-class="transition-[opacity,transform] duration-100 ease-out"
      enter-from-class="opacity-0 scale-95"
      leave-to-class="opacity-0 scale-95"
    >
      <div
        v-if="isPopoverOpen"
        ref="floatingRef"
        class="z-80 w-56 rounded-lg bg-surface px-3 py-2 text-xs leading-5 text-fg-muted ring-1 ring-line-subtle shadow-lg"
        :style="floatingStyles"
        @mouseenter="openPopover"
        @mouseleave="scheduleClosePopover"
      >
        <div class="mb-2 text-[11px] font-medium tracking-wide text-fg-body">{{ t('agent.block.responseStats') }}</div>
        <div class="grid gap-y-1">
          <div
            v-for="item in statItems"
            :key="item.label"
            class="flex items-center justify-between gap-4 font-mono"
          >
            <span class="font-sans text-fg-subtle">{{ item.label }}</span>
            <span :class="item.valueClass">{{ item.value }}</span>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
