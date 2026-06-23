import { computed, nextTick, ref, type Ref, watch } from 'vue'
import { useVirtualizer } from '@tanstack/vue-virtual'

type ScrollIntent = 'up' | 'down' | null
interface VirtualizedBlock {
  id: string
  type: string
}

const OVERSCAN = 8
const BOTTOM_THRESHOLD = 100
const AUTO_SCROLL_REENGAGE_THRESHOLD = 1

const BLOCK_HEIGHT_ESTIMATES: Record<string, number> = {
  user: 40,
  steer: 40,
  pending_steer: 40,
  resume: 0,
  thinking: 20,
  redacted_thinking: 0,
  text: 40,
  response: 24,
  tool_call: 28,
  error: 28,
  abort: 0,
  compaction_boundary: 36,
  compaction_marker: 0,
  extension_state_snapshot: 0,
}

export interface ScrollAnchor {
  blockId: string
  anchorIndex: number
  offsetPx: number
  scrollTop: number
}

export interface PersistedScrollState {
  anchor: ScrollAnchor
  heightCache: Map<string, number>
}

export function useBlockVirtualizer(
  scrollContainer: Ref<HTMLDivElement | undefined>,
  blocks: Ref<VirtualizedBlock[]>,
  persistedState: PersistedScrollState | undefined,
) {
  const heightCache = new Map<string, number>(persistedState?.heightCache ?? [])

  const scrollOffset = ref(0)
  const isAtBottom = ref(true)
  const isRestored = ref(false)

  const virtualizer = useVirtualizer<HTMLDivElement, HTMLElement>(
    computed(() => ({
      count: blocks.value.length,
      getScrollElement: () => scrollContainer.value ?? null,
      estimateSize: (index: number) => {
        const block = blocks.value[index]
        if (!block) return 40
        return heightCache.get(block.id) ?? (BLOCK_HEIGHT_ESTIMATES[block.type] ?? 40)
      },
      overscan: OVERSCAN,
      gap: 8,
      getItemKey: (index: number) => {
        const block = blocks.value[index]
        return block ? block.id : index
      },
    })),
  )

  const virtualItems = computed(() => virtualizer.value.getVirtualItems())
  const totalSize = computed(() => virtualizer.value.getTotalSize())

  function measureElement(el: Element | null) {
    if (!el) return
    const index = Number((el as HTMLElement).dataset['index'])
    const block = blocks.value[index]
    virtualizer.value.measureElement(el as HTMLElement)
    const nextMeasurement = virtualizer.value.measurementsCache[index]
    if (block && nextMeasurement) heightCache.set(block.id, nextMeasurement.size)
  }

  const shouldAutoScroll = ref(true)
  let isProgrammaticScroll = false
  let pendingIntent: ScrollIntent = null
  let touchStartY = 0
  let furthestDistanceSinceDisengage = 0

  function scrollToBottom() {
    isProgrammaticScroll = true
    virtualizer.value.scrollToIndex(blocks.value.length - 1, { align: 'end' })
    isProgrammaticScroll = false
    furthestDistanceSinceDisengage = 0
  }

  function scrollToBottomExplicit() {
    shouldAutoScroll.value = true
    scrollToBottom()
  }

  function updateAutoScrollState(dist: number, threshold: number) {
    if (pendingIntent === 'up' || !shouldAutoScroll.value) {
      furthestDistanceSinceDisengage = Math.max(furthestDistanceSinceDisengage, dist)
    }
    if (isProgrammaticScroll) return
    if (pendingIntent === 'up') {
      shouldAutoScroll.value = false
    } else if (pendingIntent === 'down' && dist <= threshold && furthestDistanceSinceDisengage > threshold) {
      shouldAutoScroll.value = true
      furthestDistanceSinceDisengage = 0
    } else if (dist <= AUTO_SCROLL_REENGAGE_THRESHOLD) {
      shouldAutoScroll.value = true
      furthestDistanceSinceDisengage = 0
    } else if (dist > threshold) {
      shouldAutoScroll.value = false
    }
  }

  watch(
    scrollContainer,
    (el, _, onCleanup) => {
      if (!el) return
      const onWheel = (e: WheelEvent) => {
        if (e.deltaY < 0) pendingIntent = 'up'
        if (e.deltaY > 0) pendingIntent = 'down'
      }
      const onTouchStart = (e: TouchEvent) => {
        touchStartY = e.touches[0]?.clientY ?? 0
      }
      const onTouchMove = (e: TouchEvent) => {
        const touchY = e.touches[0]?.clientY ?? touchStartY
        if (touchY > touchStartY) pendingIntent = 'up'
        if (touchY < touchStartY) pendingIntent = 'down'
        touchStartY = touchY
      }
      el.addEventListener('wheel', onWheel, { passive: true })
      el.addEventListener('touchstart', onTouchStart, { passive: true })
      el.addEventListener('touchmove', onTouchMove, { passive: true })
      onCleanup(() => {
        el.removeEventListener('wheel', onWheel)
        el.removeEventListener('touchstart', onTouchStart)
        el.removeEventListener('touchmove', onTouchMove)
      })
    },
    { immediate: true },
  )

  watch(
    () => virtualizer.value.getTotalSize(),
    () => {
      if (!isRestored.value || !shouldAutoScroll.value) return
      scrollToBottom()
    },
    { flush: 'post' },
  )

  watch(
    virtualizer,
    (instance) => {
      instance.shouldAdjustScrollPositionOnItemSizeChange = (item, _delta, currentInstance) => {
        const internalInstance = currentInstance as unknown as { getScrollOffset(): number; scrollAdjustments: number }
        const offset = internalInstance.getScrollOffset()
        const isStreamingTail = item.index === blocks.value.length - 1
        const allowCorrection = shouldAutoScroll.value || !isStreamingTail
        return allowCorrection && item.start < offset + internalInstance.scrollAdjustments
      }
    },
    { immediate: true },
  )

  let lastAnchor: ScrollAnchor | null = null

  function updateAnchor() {
    const el = scrollContainer.value
    if (!el || blocks.value.length === 0) return
    const st = el.scrollTop
    const containerTop = el.getBoundingClientRect().top
    for (const item of virtualizer.value.getVirtualItems()) {
      if (item.start + item.size > st) {
        const anchorEl = el.querySelector(`[data-index="${item.index}"]`) as HTMLElement | null
        if (!anchorEl) return
        lastAnchor = {
          blockId: blocks.value[item.index]!.id,
          anchorIndex: item.index,
          offsetPx: anchorEl.getBoundingClientRect().top - containerTop,
          scrollTop: st,
        }
        return
      }
    }
  }

  function onScroll() {
    const el = scrollContainer.value
    if (el) {
      scrollOffset.value = el.scrollTop
      const dist = el.scrollHeight - el.scrollTop - el.clientHeight
      isAtBottom.value = dist <= BOTTOM_THRESHOLD
      updateAutoScrollState(dist, BOTTOM_THRESHOLD)
      pendingIntent = null
    }
    updateAnchor()
  }

  function restoreScroll() {
    const el = scrollContainer.value
    if (!el || blocks.value.length === 0 || isRestored.value) return
    isRestored.value = true

    if (!persistedState) {
      scrollToBottom()
      nextTick(() => updateAnchor())
      return
    }

    const anchorIndex = blocks.value.findIndex((b) => b.id === persistedState.anchor.blockId)
    if (anchorIndex < 0) {
      scrollToBottom()
      nextTick(() => updateAnchor())
      return
    }

    el.scrollTop = persistedState.anchor.scrollTop
    correctUntilConverged(el, anchorIndex, persistedState.anchor.offsetPx, 3)
  }

  function correctUntilConverged(el: HTMLElement, anchorIndex: number, targetOffset: number, maxPasses: number) {
    waitForScrollStable(el, () => {
      const anchorEl = el.querySelector(`[data-index="${anchorIndex}"]`) as HTMLElement | null
      if (!anchorEl) {
        updateAnchor()
        return
      }
      const correction = anchorEl.getBoundingClientRect().top - el.getBoundingClientRect().top - targetOffset
      if (Math.abs(correction) > 1 && maxPasses > 0) {
        el.scrollTop += correction
        correctUntilConverged(el, anchorIndex, targetOffset, maxPasses - 1)
      } else {
        updateAnchor()
      }
    })
  }

  watch(
    () => [blocks.value.length, scrollContainer.value] as const,
    ([len, el]) => {
      if (!isRestored.value && len > 0 && el) {
        nextTick(() => restoreScroll())
      }
    },
    { immediate: true, flush: 'post' },
  )

  function getPersistedState(): PersistedScrollState | undefined {
    if (!lastAnchor) return undefined
    return {
      anchor: { ...lastAnchor, scrollTop: scrollContainer.value?.scrollTop ?? lastAnchor.scrollTop },
      heightCache: new Map(heightCache),
    }
  }

  return {
    virtualizer,
    virtualItems,
    totalSize,
    measureElement,
    scrollOffset,
    isAtBottom,
    scrollToBottom: scrollToBottomExplicit,
    onScroll,
    getPersistedState,
  }
}

function waitForScrollStable(el: HTMLElement, callback: () => void) {
  let prev = el.scrollTop
  let stable = 0
  function check() {
    if (el.scrollTop === prev) {
      if (++stable >= 2) return callback()
    } else {
      stable = 0
      prev = el.scrollTop
    }
    requestAnimationFrame(check)
  }
  requestAnimationFrame(check)
}
