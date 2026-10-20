import { messageFileTarget } from '../../markdown/filePath'
import type { MessageFiles } from '../../markdown/types'

/**
 * A click on a user message's link: in the conversation a web link opens as a link does and a
 * Host file opens in the File view; in the composer, `editable`, a click
 * places the cursor and ⌘/Ctrl+click opens either.
 */
export function followMessageLink(event: MouseEvent, editable: boolean, files: MessageFiles | undefined): void {
  const link = event.target instanceof Element ? event.target.closest('a[data-link]') : null
  if (!link) {
    return
  }
  const kind = link.getAttribute('data-link')
  const target = link.getAttribute('data-target') ?? ''
  if (kind === 'web' && !editable) {
    return
  }
  event.preventDefault()
  if (editable && !event.metaKey && !event.ctrlKey) {
    return
  }
  if (kind === 'web') {
    window.open(target, '_blank', 'noopener,noreferrer')
    return
  }
  const file = kind === 'file' && files ? messageFileTarget(target, files) : null
  if (files?.open && file !== null) {
    files.open(file)
  }
}
