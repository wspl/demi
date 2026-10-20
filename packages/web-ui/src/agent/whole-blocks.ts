import { computed, inject, provide, ref, watch, type InjectionKey, type Ref } from 'vue'
import type { HeldBlock, TextArrival } from '@demicodes/conversation-client'
import { useTranscript } from './edit-selection'

/**
 * Reads a block whole, for a row whose block the page holds in its light form:
 * the agent's (`node`, null for the conversation's own) and the block's id. The
 * host keeps the block whole once it arrives. /
 */
export type BlockReader = (node: string | null, id: string) => Promise<void>

/**
 * The block a row shows: one block, or several the page shows as one, as
 * thinking blocks next to each other, whose `parts` are the blocks it
 * joins, light while any of them is.
 */
export type ShownBlock = HeldBlock & { parts?: readonly HeldBlock[] }

const readerKey: InjectionKey<() => BlockReader | undefined> = Symbol('block-reader')
const scopeKey: InjectionKey<() => ShownBlock | undefined> = Symbol('block-scope')

/** Gives the rows below the host's way to read a block whole. */
export function provideBlockReader(reader: () => BlockReader | undefined): void {
  provide(readerKey, reader)
}

/**
 * Names the block the rows below show, for a row that opens to read it whole.
 */
export function provideBlockScope(block: () => ShownBlock | undefined): void {
  provide(scopeKey, block)
}

/**
 * When each piece of the text of the row's block reached the page, for its
 * fade; none outside a transcript.
 */
export function useTextArrivals(): () => readonly TextArrival[] | undefined {
  const scope = inject(scopeKey, () => undefined)
  return () => scope()?.arrivals
}

/**
 * The block of the row this is in, light or whole, read whole while the row
 * is `open`: `light` while the page holds it light, which makes its row open
 * even before its body is there; while the row is open, each of its parts the
 * page holds light is read whole once, a part that joins the open row as well,
 * such as the next thinking block a page of history brings, with `loading`
 * while any is on its way. A row outside a transcript, or with no host to read
 * through, is never light.
 */
export function useWholeBlock(open: () => boolean): { light: Readonly<Ref<boolean>>; loading: Readonly<Ref<boolean>> } {
  const scope = inject(scopeKey, () => undefined)
  const reader = inject(readerKey, () => undefined)
  const transcript = useTranscript()
  const reading = ref(0)
  const loading = computed(() => reading.value > 0)
  const light = computed(() => scope()?.light === true && reader() !== undefined)
  /**
   * The parts of the row's block the page holds light, by id, while the row is
   * open.
   */
  const wanted = computed(() => {
    const block = scope()
    if (!open() || !block?.light)
      return []
    return (block.parts ?? [block]).filter((part) => part.light).map((part) => part.id)
  })
  // The parts read since the row last opened: each once while it stays open.
  const asked = new Set<string>()
  watch(open, (now) => {
    if (!now)
      asked.clear()
  })
  watch(wanted, (ids) => {
    const read = reader()
    if (!read)
      return
    const node = transcript?.node ?? null
    for (const id of ids.filter((each) => !asked.has(each))) {
      asked.add(id)
      reading.value += 1
      // A read that fails leaves the row open on what it has; opening it again
      // tries again.
      read(node, id)
        .catch(() => {})
        .finally(() => {
          reading.value -= 1
        })
    }
  }, { immediate: true })
  return { light, loading }
}
