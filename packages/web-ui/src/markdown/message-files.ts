import { computed, inject, provide, type ComputedRef, type InjectionKey } from 'vue'
import { transcriptRowOf, type MediaSequence, type MediaViewerState } from '../files/media-viewer'
import { messagePicture } from './media-run'
import { HOST_FILE_ATTRIBUTE } from './render'
import type { MessageAgents } from '../agent/agent-links'
import type { MessageFiles } from './types'

const messageFilesKey: InjectionKey<() => MessageFiles | undefined> = Symbol('message-files')

/**
 * Where the messages below find the files they name
 * (`file-previews.md` § Files named in messages); read at each render, so it
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
      && previous.home === next.home
      && previous.contents === next.contents
      && !previous.open === !next.open
      && !previous.codeFiles === !next.codeFiles
      && !previous.attachment === !next.attachment
      && !previous.openCommit === !next.openCommit
    return same ? previous : next
  })
}

/**
 * A click on a message's file link opens the file, at its line when it
 * names one; one on an image the message shows, or on an attachment's image
 * or video or its link, shows it large in `viewer`, where a video plays, and
 * `among` gives the media of the transcript it stands in; an image inside a
 * link follows the link; one on an agent's link opens the agent in the
 * Agents window; one on a commit's link opens the commit in the Change
 * view; any other click is the page's.
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
  const image = target?.closest('a[data-attachment-image]')
  if (image) {
    event.preventDefault()
    viewer.show({ kind: 'image', src: image.getAttribute('href') ?? '', name: image.getAttribute('data-attachment-image') ?? '', row: transcriptRowOf(image) }, among)
    return
  }
  const video = target?.closest('a[data-attachment-video]')
  if (video) {
    event.preventDefault()
    viewer.show({ kind: 'video', src: video.getAttribute('href') ?? '', name: video.getAttribute('data-attachment-video') ?? '', row: transcriptRowOf(video) }, among)
    return
  }
  if (target instanceof HTMLImageElement && !target.closest('a')) {
    event.preventDefault()
    const { kind, src, size, caption } = messagePicture(target)
    const path = target.getAttribute(HOST_FILE_ATTRIBUTE)
    const shown = { kind, name: caption || 'image', size, row: transcriptRowOf(target) }
    // A Host image shows its file, at the version the page keeps, as the message does.
    viewer.show(path !== null && files?.contents ? { ...shown, file: { path, contents: files.contents } } : { ...shown, src }, among)
    return
  }
  const commit = target?.closest('a[data-commit-link]')
  if (commit) {
    event.preventDefault()
    files?.openCommit?.({ commit: commit.getAttribute('data-commit-link') ?? '' })
    return
  }
  const link = target?.closest('a[data-file-link]')
  if (!link || !files?.open)
    return
  event.preventDefault()
  const line = Number(link.getAttribute('data-line'))
  files.open({ path: link.getAttribute('href') ?? '', line: line > 0 ? line : null })
}

/** Return or Space on an agent's link, inline text that acts as a button, clicks it as on a button. */
export function pressMessageLink(event: KeyboardEvent): void {
  if (event.key !== 'Enter' && event.key !== ' ')
    return
  const link = event.target instanceof HTMLElement && event.target.matches('[data-agent-link][role="button"]') ? event.target : null
  if (!link)
    return
  event.preventDefault()
  link.click()
}
