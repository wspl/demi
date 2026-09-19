import { inject, provide, type InjectionKey } from 'vue'
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

/** A click on a message's file link opens the file; any other click is the page's. */
export function openFileLink(event: MouseEvent, files: MessageFiles | undefined): void {
  const link = event.target instanceof Element ? event.target.closest('a[data-file-link]') : null
  if (!link || !files)
    return
  event.preventDefault()
  files.open(link.getAttribute('href') ?? '')
}
