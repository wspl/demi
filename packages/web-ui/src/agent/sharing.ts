import { ref, shallowRef } from 'vue'
import { useCopy } from '../composables/useCopy'
import { errorCode, reportError } from '../infra/errors'
import { showToast } from '../infra/toast'
import { t } from './locales/text'

/**
 * What a share holds: the conversation, or the answer that the text block
 * `blockId` ends.
 */
export type ShareTarget = { kind: 'conversation' } | { kind: 'answer'; blockId: string }

/** A share the conversation has, as its state lists it. */
export type ExistingShare = { kind: 'conversation'; token: string } | { kind: 'answer'; token: string; blockId: string }

export interface ShareLink {
  token: string
  url: string
}

/** What the host does for the share dialog, over the owner's share routes. */
export interface ShareService {
  /**
   * The link of the share `token`, for a share the conversation holds already.
   */
  link(token: string): string
  /**
   * The conversation's or the answer's share: the existing one, or a new one.
   */
  create(target: ShareTarget): Promise<ShareLink>
  /** Moves the conversation's share to now under the same token. */
  update(token: string): Promise<ShareLink>
  /** Removes the share. */
  remove(token: string): Promise<void>
}

/** What an answer's footer needs to share it: what its button starts. */
export interface AnswerShare {
  start: () => void
}

/**
 * Whether `existing`, a share or a target, is the share of `target`: the
 * conversation's, or the answer's of that block.
 */
export function holdsTarget(existing: ShareTarget, target: ShareTarget): boolean {
  return existing.kind === target.kind && (existing.kind === 'conversation' || (target.kind === 'answer' && existing.blockId === target.blockId))
}

/**
 * What the share dialog shows: the link being made, the link, the link
 * being moved to now by Update Link, or why making it failed.
 */
export type SharePhase =
  | { status: 'creating' }
  | { status: 'ready'; token: string; url: string }
  | { status: 'updating'; token: string; url: string }
  | { status: 'failed'; error: unknown }

/** The share the dialog shows: its target and its phase. */
export type ShownShare = { target: ShareTarget } & SharePhase

/**
 * The flow of the share dialog, one for a session.
 *
 * - `start(target)` opens the dialog at once: on the share the conversation has
 *   already, or on a new one it makes there, Creating link… until its link
 *   stands, already copied.
 * - A failure to make it stays in the dialog with Try Again (`retry`).
 * - The dialog's Copy Link, Update Link and Stop Sharing act on the share
 *   shown, and their failures are toasts.
 * - The host's `service` answers each step; absent, nothing can be shared.
 *
 * `start` and `retry` run inside the click that asks for the link: they put the
 * link on the clipboard there, as an item whose text is still being made, since
 * Safari lets a page write the clipboard only from a click and not after the
 * request it waits for.
 */
export function useShare(service: () => ShareService | undefined, existing: () => readonly ExistingShare[]) {
  /** The share the dialog shows; kept while it fades out. */
  const shown = shallowRef<ShownShare | null>(null)
  const isOpen = ref(false)
  const { copy, copyWhenMade, copied } = useCopy(() => (shown.value?.status === 'ready' ? shown.value.url : undefined))

  /**
   * Makes the share of `target` in the dialog and copies its link once it
   * stands.
   */
  function make(host: ShareService, target: ShareTarget): void {
    const attempt: ShownShare = { target, status: 'creating' }
    shown.value = attempt
    const made = host.create(target)
    void copyWhenMade(made.then((link) => link.url))
    made.then(
      (link) => {
        // A later Share replaced what the dialog shows: this answer is no
        // one's.
        if (shown.value === attempt) {
          shown.value = { target, status: 'ready', token: link.token, url: link.url }
        }
      },
      (error: unknown) => {
        reportError(target.kind === 'conversation' ? t('share.failedConversation') : t('share.failedAnswer'), error)
        if (shown.value === attempt) {
          shown.value = { target, status: 'failed', error }
        }
      },
    )
  }

  function start(target: ShareTarget): void {
    const host = service()
    if (!host) {
      return
    }
    isOpen.value = true
    const current = shown.value
    if (current?.status === 'creating' && holdsTarget(current.target, target)) {
      // Its link is still being made: the dialog shows that one.
      return
    }
    const held = existing().find((share) => holdsTarget(share, target))
    if (held) {
      shown.value = { target, status: 'ready', token: held.token, url: host.link(held.token) }
      return
    }
    make(host, target)
  }

  /** Try Again, after making the link failed. */
  function retry(): void {
    const host = service()
    const current = shown.value
    if (host && current?.status === 'failed') {
      make(host, current.target)
    }
  }

  async function update(): Promise<void> {
    const host = service()
    const current = shown.value
    if (!host || current?.status !== 'ready' || current.target.kind !== 'conversation') {
      return
    }
    const updating: ShownShare = { ...current, status: 'updating' }
    shown.value = updating
    try {
      await host.update(current.token)
    } catch (error) {
      reportError(t('share.failedUpdate'), error, { userVisible: true })
      return
    } finally {
      if (shown.value === updating) {
        shown.value = current
      }
    }
    showToast({ title: t('share.updated'), message: t('share.updatedMessage'), tone: 'neutral' })
  }

  async function stop(): Promise<void> {
    const host = service()
    const current = shown.value
    if (!host || current?.status !== 'ready') {
      return
    }
    try {
      await host.remove(current.token)
    } catch (error) {
      // A share another page stopped meanwhile is stopped: that is done.
      if (errorCode(error) !== 'share_not_found') {
        reportError(t('share.failedStop'), error, { userVisible: true })
        return
      }
    }
    isOpen.value = false
  }

  return { shown, isOpen, copied, start, retry, update, stop, copy, close: () => { isOpen.value = false } }
}

