import type { ShownEntry } from './file-cache'
import type { FileContents, FileDescription } from './types'

/**
 * What shows of an image at a Host path: still being read, the version
 * the page keeps of it, or nothing, as for a file that is not there or a
 * Host that cannot be read, which a message shows as its alt text.
 */
export type HostImage =
  | { state: 'loading' }
  | { state: 'shown'; src: string }
  | { state: 'none' }

/**
 * What shows of the Host image at `path`, from the description of its file
 * the page keeps (`entry`): its bytes at the version the description names,
 * so a new version is a new address the web browser loads, never one it
 * answers from the picture it already decoded for the same address
 * (`file-previews.md` § Files named in messages).
 */
export function hostImage(
  entry: ShownEntry<FileDescription> | null | undefined,
  contents: FileContents | undefined,
  path: string,
): HostImage {
  if (!contents)
    return { state: 'none' }
  const description = entry?.value
  if (entry?.answer === 'value' && description !== undefined)
    return { state: 'shown', src: contents.url(path, description.version === null ? {} : { version: description.version }) }
  if (!entry || (entry.answer === null && entry.failure === null))
    return { state: 'loading' }
  return { state: 'none' }
}
