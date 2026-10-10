import { computed, inject, provide, ref, type InjectionKey, type Ref } from 'vue'
import type { HeldBlock } from '@demicodes/conversation-client'
import { useTranscript } from './edit-selection'

/**
 * Reads a block whole, for a row whose block the page holds in its light
 * form (`web-api.md` § Light form): the agent's (`node`, null for the
 * conversation's own) and the block's id. The host keeps the block whole
 * once it arrives.
 */
export type BlockReader = (node: string | null, id: string) => Promise<void>

const readerKey: InjectionKey<() => BlockReader | undefined> = Symbol('block-reader')
const scopeKey: InjectionKey<() => HeldBlock | undefined> = Symbol('block-scope')

/** Gives the rows below the host's way to read a block whole. */
export function provideBlockReader(reader: () => BlockReader | undefined): void {
  provide(readerKey, reader)
}

/** Names the block the rows below show, for a row that opens to read it whole. */
export function provideBlockScope(block: () => HeldBlock | undefined): void {
  provide(scopeKey, block)
}

/**
 * The block of the row this is in, light or whole, and the way to open it
 * whole (`web-application.md` § Transcript windows): `light` while the page
 * holds it light, which makes its row open even before its body is there;
 * `open()` reads it whole once, with `loading` while it is on its way. A
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
    // A read that fails leaves the row open on what it has; opening it again tries again.
    read(transcript?.node ?? null, block.id)
      .catch(() => {})
      .finally(() => {
        loading.value = false
      })
  }
  return { light, loading, open }
}
