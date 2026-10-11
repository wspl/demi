import { messageCodePaths } from './code-paths'
import { messageCodeSpans } from './render'
import type { MessageFiles } from './types'

/**
 * Which of the Host paths a reply's inline code names are files there
 * (`file-previews.md` § Files named in messages): asked of the host once
 * the reply is complete, never while it streams, for all its paths in one
 * question; null until the host answered, and when the reply names none.
 */
export function replyCodeFiles(
  content: string,
  streaming: boolean,
  files: MessageFiles | undefined,
): ReadonlySet<string> | null {
  if (streaming || !files?.open || !files.codeFiles)
    return null
  const paths = messageCodePaths(messageCodeSpans(content), files.cwd)
  return paths.length > 0 ? files.codeFiles(paths) : null
}
