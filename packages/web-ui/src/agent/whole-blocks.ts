import { computed, inject, provide, ref, type InjectionKey, type Ref } from 'vue'
import type { HeldBlock, TextArrival } from '@demicodes/conversation-client'
import { useTranscript } from './edit-selection'

/**
 * Reads a block whole, for a row whose block the page holds in its light
 * form (`web-api.md` § Light form): the agent's (`node`, null for the
 * conversation's own) and the block's id. The host keeps the block whole
 * once it arrives.
 */
export type BlockReader = (node: string | null, id: string) => Promise<void>

/**
 * The block a row shows: one block, or several the page shows as one, as
 * thinking blocks next to each other (`runtime.md` § Work groups), whose
 * `parts` are the blocks it joins, light while any of them is.
 */
export type ShownBlock = HeldBlock & { parts?: readonly HeldBlock[] }

const readerKey: InjectionKey<() => BlockReader | undefined> = Symbol('block-reader')
const scopeKey: InjectionKey<() => ShownBlock | undefined> = Symbol('block-scope')

/** Gives the rows below the host's way to read a block whole. */
export function provideBlockReader(reader: () => BlockReader | undefined): void {
  provide(readerKey, reader)
}

/** Names the block the rows below show, for a row that opens to read it whole. */
export function provideBlockScope(block: () => ShownBlock | undefined): void {
  provide(scopeKey, block)
}

/** When each piece of the text of the row's block reached the page, for its fade; none outside a transcript. */
export function useTextArrivals(): () => readonly TextArrival[] | undefined {
  const scope = inject(scopeKey, () => undefined)
  return () => scope()?.arrivals
}

/**
 * The block of the row this is in, light or whole, and the way to open it
 * whole (`web-application.md` § Transcript windows): `light` while the page
 * holds it light, which makes its row open even before its body is there;
 * `open()` reads it whole once, each of its light parts for a row that
 * shows several blocks as one, with `loading` while it is on its way. A
 * row outside a transcript, or with no host to read through, is never light.
 */
export function useWholeBlock(): { light: Readonly<Ref<boolean>>; loading: Readonly<Ref<boolean>>; open: () => void } {
  const scope = inject(scopeKey, () => undefined)
  const reader = inject(readerKey, () => undefined)
  const transcript = useTranscript()
  const loading = ref(false)
  const light = computed(() => scope()?.light === true && reader() !== undefined)
  function open(): void {
    const block = scope()
    const read = reader()
    if (!block?.light || !read || loading.value) {
      return
    }
    loading.value = true
    const node = transcript?.node ?? null
    // A read that fails leaves the row open on what it has; opening it again tries again.
    Promise.all((block.parts ?? [block]).filter((part) => part.light).map((part) => read(node, part.id)))
      .catch(() => {})
      .finally(() => {
        loading.value = false
      })
  }
  return { light, loading, open }
}
