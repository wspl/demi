import { computed, inject, provide, type ComputedRef, type InjectionKey } from 'vue'
import { transcriptRowOf, type MediaSequence, type MediaViewerState, type ShownMedium } from '../files/media-viewer'
import { messagePicture } from './media-run'
import { ATTACHMENT_ID_ATTRIBUTE, HOST_FILE_ATTRIBUTE, attachmentMedium, messageAttachment } from './render'
import type { MessageAgents } from '../agent/agent-links'
import { agentMenu } from '../agent/agent-menu'
import { downloadUrl } from '../files/download'
import { commitMenu, fileMenu } from '../files/object-menus'
import type { ContextMenuGroup } from '../ui/context-menu'
import type { MessageCommitTarget, MessageFiles } from './types'

const messageFilesKey: InjectionKey<() => MessageFiles | undefined> = Symbol('message-files')

/**
 * Where the messages below find the files they name; read at each render, so it
 * follows the conversation.
 */
export function provideMessageFiles(files: () => MessageFiles | undefined): void {
  provide(messageFilesKey, files)
}

/** The conversation's files, or none outside a conversation. */
export function useMessageFiles(): () => MessageFiles | undefined {
  return inject(messageFilesKey, () => undefined)
}

/**
 * `files` as a render reads them: the same object while it reads the same
 * of them, its directory, its Host's home, its kept files and which of its
 * services there are. Their owners make a new object each time the
 * conversation changes, which is each time a reply's text grows, and a
 * streaming reply's settled blocks would render again with each
 * (`useMarkdownBlocks`). The services read what they read when called, so
 * the first object's serve as well as the next's.
 */
export function useRenderedFiles(files: () => MessageFiles | undefined): ComputedRef<MessageFiles | undefined> {
  return computed((previous) => {
    const next = files()
    const same = previous !== undefined && next !== undefined
      && previous.cwd === next.cwd
      && previous.node === next.node
      && previous.home === next.home
      && previous.contents === next.contents
      && !previous.open === !next.open
      && !previous.attachment === !next.attachment
      && !previous.openCommit === !next.openCommit
    return same ? previous : next
  })
}

/**
 * A click on a message's file link opens the file, at its line when it names
 * one. Other clicks:
 *
 * - A click on an image the message shows, or on an attachment's image or video
 *   or its link, shows it large in `viewer`, where a video plays: an
 *   attachment's file while it opens in place of its copy (`attachmentMedium`).
 *   `among` gives the media of the transcript it stands in.
 * - An image inside a link follows the link.
 * - A click on an agent's link opens the agent in the Agents window.
 * - A click on a commit's link opens the commit in the Change view.
 * - Any other click is the page's.
 */
export function openMessageLink(
  event: MouseEvent,
  files: MessageFiles | undefined,
  viewer: MediaViewerState,
  agents: MessageAgents | undefined,
  among?: MediaSequence,
): void {
  const target = event.target instanceof Element ? event.target : null
  const agent = target?.closest('[data-agent-link][role="button"]')
  if (agent && agents) {
    event.preventDefault()
    agents.open(Number(agent.getAttribute('data-agent-link')))
    return
  }
  const tile = target?.closest('a[data-attachment-image], a[data-attachment-video]')
  if (tile) {
    event.preventDefault()
    viewer.show({ ...tileMedium(tile, files), row: transcriptRowOf(tile) }, among)
    return
  }
  if (target instanceof HTMLImageElement && !target.closest('a')) {
    event.preventDefault()
    const { kind, src, size, caption } = messagePicture(target)
    const path = target.getAttribute(HOST_FILE_ATTRIBUTE)
    const shown = { kind, name: caption || 'image', size, row: transcriptRowOf(target) }
    // A Host image shows its file, at the version the page keeps, as the
    // message does.
    viewer.show(path !== null && files?.contents ? { ...shown, file: { path, contents: files.contents } } : { ...shown, src }, among)
    return
  }
  const commit = target?.closest('a[data-commit-link]')
  if (commit) {
    event.preventDefault()
    files?.openCommit?.(commitTarget(commit))
    return
  }
  const link = target?.closest('a[data-file-link]')
  if (!link || !files?.open)
    return
  event.preventDefault()
  const line = Number(link.getAttribute('data-line'))
  files.open({ path: link.getAttribute('href') ?? '', line: line > 0 ? line : null })
}

/**
 * The medium an attachment's tile shows large: the attachment as its number
 * names it now (`attachmentMedium`), or the copy the tile links while the
 * page no longer has it.
 */
function tileMedium(tile: Element, files: MessageFiles | undefined): ShownMedium {
  const id = tile.getAttribute(ATTACHMENT_ID_ATTRIBUTE)
  const lookup = id === null ? undefined : messageAttachment(files, id)
  const medium = id !== null && lookup?.state === 'found' ? attachmentMedium(id, lookup.attachment, files) : null
  if (medium)
    return medium
  const video = tile.getAttribute('data-attachment-video')
  return video === null
    ? { kind: 'image', src: tile.getAttribute('href') ?? '', name: tile.getAttribute('data-attachment-image') ?? '' }
    : { kind: 'video', src: tile.getAttribute('href') ?? '', name: video }
}

/**
 * The commit a rendered commit link names, with the folder of its repository
 * when the link names one.
 */
function commitTarget(link: Element): MessageCommitTarget {
  const commit = link.getAttribute('data-commit-link') ?? ''
  const root = link.getAttribute('data-commit-root')
  return root === null ? { commit } : { commit, root }
}

/**
 * A commit's id written whole, SHA-1's or SHA-256's, which needs no reading to
 * be the full hash.
 */
const FULL_HASH = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/

/**
 * The menu of the object a message's link names, the one its row or tab has,
 * Open first, which clicks the link: a file's, a commit's or an agent's, from
 * the conversation's `files` and `agents` as they are when it opens. Null for
 * any other link, such as an attachment's, whose click is its Open.
 */
export function messageLinkMenu(
  link: Element,
  files: MessageFiles | undefined,
  agents: MessageAgents | undefined,
): (() => readonly ContextMenuGroup[]) | null {
  if (!(link instanceof HTMLElement))
    return null
  const open = () => link.click()
  if (link.matches('[data-agent-link][role="button"]') && agents) {
    const number = Number(link.getAttribute('data-agent-link'))
    return () => agentMenu(
      { number, running: agents.find(number)?.run?.phase === 'running' },
      { open, stop: () => agents.stop(number) },
    )
  }
  if (link.matches('a[data-commit-link]')) {
    const target = commitTarget(link)
    const resolve = files?.commitHash
    const hash = FULL_HASH.test(target.commit) ? () => target.commit : resolve ? () => resolve(target) : undefined
    return () => commitMenu({ open, ...(hash ? { hash } : {}) })
  }
  if (link.matches('a[data-file-link]')) {
    const path = link.getAttribute('href') ?? ''
    const contents = files?.contents
    const showInFiles = files?.showInFiles
    return () => fileMenu(path, {
      open,
      ...(showInFiles ? { showInFiles: () => showInFiles({ path }) } : {}),
      ...(contents ? { download: () => downloadUrl(contents.url(path, { download: true })) } : {}),
    })
  }
  return null
}

/**
 * Return or Space on an agent's link, inline text that acts as a button, clicks
 * it as on a button.
 */
export function pressMessageLink(event: KeyboardEvent): void {
  if (event.key !== 'Enter' && event.key !== ' ')
    return
  const link = event.target instanceof HTMLElement && event.target.matches('[data-agent-link][role="button"]') ? event.target : null
  if (!link)
    return
  event.preventDefault()
  link.click()
}
