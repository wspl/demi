import { ref, shallowRef } from 'vue'
import { useCopy } from '../composables/useCopy'
import { reportError } from '../infra/errors'
import { t } from './locales/text'
import type { SettingsShareKey } from './types'

/** What the host does for Share Provider, over the entry's share-key routes. */
export interface ProviderShareService {
  list(providerId: string): Promise<SettingsShareKey[]>
  /** A new key, whose text only this answer holds. */
  create(providerId: string): Promise<{ key: SettingsShareKey; text: string }>
  revoke(providerId: string, keyId: string): Promise<void>
}

/** The provider's keys as the dialog lists them. */
export type ShareKeysPhase =
  | { status: 'loading' }
  | { status: 'ready'; keys: SettingsShareKey[] }
  | { status: 'failed'; error: unknown }

/** The key the dialog made and shows the one time: being made, made, or why not. */
export type MadeKeyPhase =
  | { status: 'creating' }
  | { status: 'ready'; id: string; text: string }
  | { status: 'failed'; error: unknown }

/** The provider Share Provider shows. */
export interface SharedProvider {
  id: string
  name: string
}

/**
 * The flow of Share Provider, one for the page.
 *
 * - `start(provider)` opens the dialog at once and lists the provider's keys;
 *   a provider with none gets one there, as Share Conversation makes its
 *   link, already copied.
 * - New Key (`make`) makes another, shown and copied the same way.
 * - The key made stays in the dialog until it closes: no later read gives it
 *   again.
 * - Revoke (`revoke`) drops the key from the list; its failure is a toast.
 *
 * `start` and `make` run inside the click that asks: they put the key on the
 * clipboard there, as an item whose text is still being made, since Safari lets
 * a page write the clipboard only from a click (`useCopy`).
 */
export function useProviderShare(service: () => ProviderShareService | undefined) {
  const isOpen = ref(false)
  /** The provider shown; kept while the dialog fades out. */
  const provider = shallowRef<SharedProvider | null>(null)
  const keys = shallowRef<ShareKeysPhase>({ status: 'loading' })
  const made = shallowRef<MadeKeyPhase | null>(null)
  const { copy, copyWhenMade, copied } = useCopy(() => (made.value?.status === 'ready' ? made.value.text : undefined))

  /**
   * Shows the key `making` makes and lists it first; `copying` puts it on the
   * clipboard once it stands, unless the click that asked did already.
   */
  function show(shown: SharedProvider, making: Promise<{ key: SettingsShareKey; text: string }>, copying: boolean): void {
    const attempt: MadeKeyPhase = { status: 'creating' }
    made.value = attempt
    if (copying) {
      void copyWhenMade(making.then((answer) => answer.text))
    }
    making.then(
      (answer) => {
        if (made.value !== attempt || provider.value !== shown) {
          return
        }
        made.value = { status: 'ready', id: answer.key.id, text: answer.text }
        const listed = keys.value.status === 'ready' ? keys.value.keys : []
        keys.value = { status: 'ready', keys: [answer.key, ...listed.filter((key) => key.id !== answer.key.id)] }
      },
      (error: unknown) => {
        reportError(t('providers.share.makeFailed'), error, { site: 'providers.share.makeFailed' })
        if (made.value === attempt) {
          made.value = { status: 'failed', error }
        }
      },
    )
  }

  function start(shown: SharedProvider): void {
    const device = service()
    if (!device) {
      return
    }
    provider.value = shown
    isOpen.value = true
    keys.value = { status: 'loading' }
    made.value = null
    const listed = device.list(shown.id)
    // A provider without keys gets its first one now, inside this click.
    const first = listed.then((list) => (list.length ? null : device.create(shown.id)))
    const making = first.then((answer) => answer ?? Promise.reject(new Error('The provider has keys: none is made')))
    making.catch(() => undefined)
    listed.then(
      (list) => {
        if (provider.value !== shown) {
          return
        }
        keys.value = { status: 'ready', keys: list }
        if (!list.length) {
          show(shown, making, false)
        }
      },
      (error: unknown) => {
        if (provider.value === shown) {
          keys.value = { status: 'failed', error }
        }
      },
    )
    // The clipboard is written in this click; it takes the text only when
    // a key is made.
    void copyWhenMade(making.then((answer) => answer.text))
  }

  /** Reads the keys again, after the list failed. */
  function reload(): void {
    const device = service()
    const shown = provider.value
    if (!device || !shown) {
      return
    }
    keys.value = { status: 'loading' }
    device.list(shown.id).then(
      (list) => {
        if (provider.value === shown) {
          keys.value = { status: 'ready', keys: list }
        }
      },
      (error: unknown) => {
        if (provider.value === shown) {
          keys.value = { status: 'failed', error }
        }
      },
    )
  }

  /** New Key. */
  function make(): void {
    const device = service()
    const shown = provider.value
    if (device && shown) {
      show(shown, device.create(shown.id), true)
    }
  }

  async function revoke(keyId: string): Promise<void> {
    const device = service()
    const shown = provider.value
    if (!device || !shown) {
      return
    }
    try {
      await device.revoke(shown.id, keyId)
    } catch (error) {
      reportError(t('providers.share.revokeFailed'), error, { site: 'providers.share.revokeFailed', userVisible: true })
      return
    }
    if (keys.value.status === 'ready' && provider.value === shown) {
      keys.value = { status: 'ready', keys: keys.value.keys.filter((key) => key.id !== keyId) }
    }
    if (made.value?.status === 'ready' && made.value.id === keyId) {
      made.value = null
    }
  }

  return { isOpen, provider, keys, made, copied, start, reload, make, revoke, copy, close: () => { isOpen.value = false } }
}
