import { computed, onBeforeUnmount, ref, watch, type ComputedRef, type Ref } from 'vue'
import {
  ACTIVITY_HANDOFF_MS,
  isHandoffBlock,
  type ActivityKind,
  type HandoffBlock,
} from './activity-slot'
import type { MessageListBlock } from './pending-steers'

export interface ActivitySlotState {
  kind: ActivityKind
  /** The block rolling into the row; the row shows its face until the transcript takes it. */
  incoming: HandoffBlock | null
}

interface Hold {
  id: string
  /** The face the row showed when the block arrived; it stays the row's kind until the handoff ends. */
  kind: ActivityKind
}

/**
 * A thinking or tool block that arrives while the tail row is waiting does not
 * replace the row: it rolls in as the row's face, and after the roll the
 * transcript row takes over in the same place. `heldId` is the block the list
 * leaves out meanwhile. The hold ends on its timer, when the block leaves the
 * tail (withdrawn or followed by another), when the scope changes, and on
 * unmount; every path releases through one function.
 */
export function useActivityHandoff(
  tail: () => MessageListBlock | undefined,
  baseKind: () => ActivityKind | null,
  scope: () => string,
): {
  heldId: ComputedRef<string | null>
  slot: ComputedRef<ActivitySlotState | null>
} {
  const hold: Ref<Hold | null> = ref(null)
  let timer: ReturnType<typeof setTimeout> | undefined

  function release(): void {
    if (timer !== undefined) {
      clearTimeout(timer)
      timer = undefined
    }
    hold.value = null
  }

  function start(id: string, kind: ActivityKind): void {
    release()
    hold.value = { id, kind }
    timer = setTimeout(() => {
      timer = undefined
      hold.value = null
    }, ACTIVITY_HANDOFF_MS)
  }

  watch(
    () => [tail()?.id, baseKind()] as const,
    ([tailId], [previousTailId, previousKind]) => {
      if (hold.value && hold.value.id !== tailId) {
        release()
      }
      if (tailId === previousTailId || !previousKind) {
        return
      }
      const block = tail()
      if (isHandoffBlock(block)) {
        start(block.id, previousKind)
      }
    },
  )

  watch(scope, release)
  onBeforeUnmount(release)

  const heldId = computed(() => hold.value?.id ?? null)
  const slot = computed<ActivitySlotState | null>(() => {
    const current = hold.value
    if (current) {
      const block = tail()
      const incoming = isHandoffBlock(block) && block.id === current.id ? block : null
      return { kind: current.kind, incoming }
    }
    const kind = baseKind()
    return kind ? { kind, incoming: null } : null
  })

  return { heldId, slot }
}
