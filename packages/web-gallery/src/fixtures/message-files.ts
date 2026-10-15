import type { MemoryDirectory, MemoryFileSource } from '@demicodes/web-ui/files/memory-source'
import type { ConversationFiles } from '@demicodes/web-ui/markdown/types'
import { galleryAttachment } from './attachments'
import { LINKED_COMMIT } from './history'

/** Whether `path` is a file of the memory tree `root`, as a Host's stat says. */
function isFile(root: MemoryDirectory, path: string): boolean {
  let node: MemoryDirectory['children'][string] | undefined = root
  for (const part of path.split('/').filter(Boolean)) {
    node = node?.kind === 'directory' ? node.children[part] : undefined
  }
  return node?.kind === 'file'
}

/**
 * Where a gallery transcript's messages find the files they name, as a
 * conversation's do: images at the workspace's paths load from `source`,
 * attachments from the gallery's blobs, and a file a click opens goes to
 * `open`, and a commit a link names to `openCommit`. The Host's answer about a reply's inline code paths is the
 * gallery's: the files of its demo workspace, answered at once.
 */
export function galleryConversationFiles(
  source: MemoryFileSource,
  open: NonNullable<ConversationFiles['open']>,
  openCommit: NonNullable<ConversationFiles['openCommit']>,
): ConversationFiles {
  return {
    openCommit,
    home: source.home,
    contents: source.contents,
    attachment: galleryAttachment,
    codeFiles: (paths) => new Set(paths.filter((path) => isFile(source.root, path))),
    open,
  }
}

/**
 * A reply that names the workspace's files: links, one at a line, and
 * paths written as code, of which the workspace has some and not others.
 */
export const namedFilesMarkdown = `The cookie is renamed in [cookie.ts](src/auth/cookie.ts), and [session.ts](src/auth/session.ts#L5) reads it under the new name. The setup steps the [README](README.md#L42) lists still hold; from the home it is [~/Projects/demi/README.md](~/Projects/demi/README.md). The fix went in as [Fix the cart total when a coupon empties it](commit:${LINKED_COMMIT}).

在中文句子里，字符与图标之间同样留出间距：已经提交了[修正优惠券清空后的购物车合计](commit:${LINKED_COMMIT})，请查看。

Written as code, \`src/auth/cookie.ts\` and \`tests/login/auth.test.ts\` open, and \`README.md:42\` opens at its line; \`src/auth/token.ts\` is not in the workspace and stays code, and a bare name such as \`cookie\` is never asked about.`
