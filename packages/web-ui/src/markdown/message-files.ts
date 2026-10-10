import { inject, provide, type InjectionKey } from 'vue'
import type { MediaViewerState } from '../files/media-viewer'
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
 * A click on a message's file link opens the file, and one on an attachment's
 * image or video, or its link, shows it large in `viewer`, where a video
 * plays; any other click is the page's.
 */
export function openMessageLink(
  event: MouseEvent,
  files: MessageFiles | undefined,
  viewer: MediaViewerState,
): void {
  const target = event.target instanceof Element ? event.target : null
  const image = target?.closest('a[data-attachment-image]')
  if (image) {
    event.preventDefault()
    viewer.show({ kind: 'image', src: image.getAttribute('href') ?? '', name: image.getAttribute('data-attachment-image') ?? '' })
    return
  }
  const video = target?.closest('a[data-attachment-video]')
  if (video) {
    event.preventDefault()
    viewer.show({ kind: 'video', src: video.getAttribute('href') ?? '', name: video.getAttribute('data-attachment-video') ?? '' })
    return
  }
  const link = target?.closest('a[data-file-link]')
  if (!link || !files?.open)
    return
  event.preventDefault()
  files.open(link.getAttribute('href') ?? '', event.detail)
}
