/**
 * What a symbolic link is now, read when it is expanded or opened
 * (`file-previews.md` § Symbolic links): a row says of its target what its
 * folder's listing said, which the folder's watch does not change when the
 * target alone does, so a link that is used has its folder listed again,
 * which reads each link's target, and the caller acts on what that found.
 */
import { baseName } from '@demicodes/utils'
import { parentPath } from './paths'
import type { FileBrowserEntry, FileBrowserSource } from './types'

/**
 * Lists the folder of the link at `path` again, joining a listing of it on
 * its way, and answers what the listing now says is at `path`: undefined
 * when it names nothing there any more, when the listing failed, or when
 * `signal` was aborted meanwhile, as by the view that asked going away, so
 * the caller does nothing with it. The listing is held only for the read.
 */
export async function readLinkAgain(
  source: Pick<FileBrowserSource, 'showListing'>,
  path: string,
  signal: AbortSignal,
): Promise<FileBrowserEntry | undefined> {
  const showing = source.showListing(parentPath(path))
  try {
    await showing.retry()
    if (signal.aborted)
      return undefined
    return showing.entry.value?.find((entry) => entry.name === baseName(path))
  } finally {
    showing.release()
  }
}
