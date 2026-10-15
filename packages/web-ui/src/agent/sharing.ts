import { ref, shallowRef } from 'vue'
import { useCopy } from '../composables/useCopy'
import { reportError } from '../infra/errors'
import { showToast } from '../infra/toast'
import type { Operation } from '../ui/operations'
import { t } from './locales/text'

/** What a share holds: the conversation, or the answer that the text block `blockId` ends (`sharing.md`). */
export type ShareTarget = { kind: 'conversation' } | { kind: 'answer'; blockId: string }

/** A share the conversation has, as its state lists it (`web-api.md` § Sharing). */
export type ExistingShare = { kind: 'conversation'; token: string } | { kind: 'answer'; token: string; blockId: string }

/** A share's link. */
export interface ShareLink {
  token: string
  url: string
}

/** What the host does for the share dialog, over the owner's share routes. */
export interface ShareService {
  /** The link of the share `token`, for a share the conversation holds already. */
  link(token: string): string
  /** The conversation's or the answer's share: the existing one, or a new snapshot. */
  create(target: ShareTarget): Promise<ShareLink>
  /** Takes the conversation's snapshot again under the same token. */
  update(token: string): Promise<ShareLink>
  /** Removes the share. */
  remove(token: string): Promise<void>
}

/** The operation of Share on `target`: its button works while the share is made, and starts nothing twice. */
export function shareOperation(target: ShareTarget): Operation {
  return { action: 'share', target: target.kind === 'conversation' ? 'share:conversation' : `share:answer:${target.blockId}` }
}

/** What an answer's footer needs to share it: the operation its button shows, and what it starts. */
export interface AnswerShare {
  operation: Operation
  start: () => Promise<void>
}

/** Whether `existing` is the share of `target`: the conversation's, or the answer's of that block. */
export function holdsTarget(existing: ExistingShare, target: ShareTarget): boolean {
  return existing.kind === target.kind && (existing.kind === 'conversation' || (target.kind === 'answer' && existing.blockId === target.blockId))
}

/**
 * The flow of the share dialog (`sharing.md` § Making, updating and removing
 * a share), one for a session: `start(target)` opens the dialog on the
 * share the conversation has already, or makes one at once and copies its
 * link, then opens it; the dialog's Copy Link, Update Link and Stop Sharing
 * act on the share shown. Every failure is a toast, and the dialog stays
 * where it is. The host's `service` answers each step; absent, nothing can
 * be shared.
 */
export function useShare(service: () => ShareService | undefined, existing: () => readonly ExistingShare[]) {
  /** The share the dialog shows; kept while it fades out. */
  const shown = shallowRef<{ target: ShareTarget; token: string; url: string } | null>(null)
  const isOpen = ref(false)
  const { copy, copied } = useCopy(() => shown.value?.url)

  async function start(target: ShareTarget): Promise<void> {
    const host = service()
    if (!host) {
      return
    }
    const held = existing().find((share) => holdsTarget(share, target))
    if (held) {
      shown.value = { target, token: held.token, url: host.link(held.token) }
      isOpen.value = true
      return
    }
    let link: ShareLink
    try {
      link = await host.create(target)
    } catch (error) {
      reportError(target.kind === 'conversation' ? t('share.failedConversation') : t('share.failedAnswer'), error, { userVisible: true })
      return
    }
    shown.value = { target, token: link.token, url: link.url }
    isOpen.value = true
    await copy()
  }

  async function update(): Promise<void> {
    const host = service()
    const current = shown.value
    if (!host || !current || current.target.kind !== 'conversation') {
      return
    }
    try {
      await host.update(current.token)
    } catch (error) {
      reportError(t('share.failedUpdate'), error, { userVisible: true })
      return
    }
    showToast({ title: t('share.updated'), message: t('share.updatedMessage'), tone: 'neutral' })
  }

  async function stop(): Promise<void> {
    const host = service()
    const current = shown.value
    if (!host || !current) {
      return
    }
    try {
      await host.remove(current.token)
    } catch (error) {
      reportError(t('share.failedStop'), error, { userVisible: true })
      return
    }
    isOpen.value = false
  }

  return { shown, isOpen, copied, start, update, stop, copy, close: () => { isOpen.value = false } }
}
