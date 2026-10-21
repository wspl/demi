import type { CodeFile } from '@demicodes/protocol'
import type { MemoryDirectory, MemoryFileSource } from '@demicodes/web-ui/files/memory-source'
import type { ConversationFiles } from '@demicodes/web-ui/markdown/types'
import { baseName } from '@demicodes/utils'
import { readCommitHash } from '@demicodes/web-ui/files/history'
import { productWould } from '../product-would'
import { GALLERY_HOST, galleryAttachment } from './attachments'
import { LINKED_COMMIT, sharedHistory } from './history'

/** Whether `path` is a file of the memory tree `root`, as a Host's stat says. */
function isFile(root: MemoryDirectory, path: string): boolean {
  let node: MemoryDirectory['children'][string] | undefined = root
  for (const part of path.split('/').filter(Boolean)) {
    node = node?.kind === 'directory' ? node.children[part] : undefined
  }
  return node?.kind === 'file'
}

/**
 * Which file an attachment opens in place of its copy, as the product
 * answers: the one it was uploaded from while the gallery's Host is online,
 * as `online` says, and the workspace has the file. None holds its copy's
 * bytes, since the gallery's files are not its blobs.
 */
export function galleryAttachmentOrigin(
  source: MemoryFileSource,
  online: () => boolean,
): NonNullable<ConversationFiles['attachmentOrigin']> {
  return async (_id, { source: origin }) =>
    origin && origin.host === GALLERY_HOST && online() && isFile(source.root, origin.path) ? { path: origin.path, sameAsCopy: false } : null
}

/**
 * Where a gallery transcript's messages find the files they name, as a
 * conversation's do:
 *
 * - Images at the workspace's paths load from `source`.
 * - Attachments load from the gallery's blobs.
 * - A file a click opens goes to `open`, and a commit a link names to
 *   `openCommit`.
 * - The Host's answer about a reply's inline code paths is the gallery's: the
 *   files of its demo workspace, answered at once.
 * - A file link's Show in Files goes to `showInFiles`, which outside a panel
 *   says what the product would do.
 * - A commit link's Copy Commit Hash reads the gallery's one repository.
 * - An attachment block's card opens its file through `openAttachment`, which
 *   outside a panel says what the product would do.
 */
export function galleryConversationFiles(
  source: MemoryFileSource,
  open: NonNullable<ConversationFiles['open']>,
  openCommit: NonNullable<ConversationFiles['openCommit']>,
  showInFiles: NonNullable<ConversationFiles['showInFiles']> = ({ path }) => productWould(`Show ${baseName(path)} in the File View’s Tree`),
  openAttachment: NonNullable<ConversationFiles['openAttachment']> = (_id, { name }) => productWould(`Open ${name} in the Work Panel`),
): ConversationFiles {
  return {
    openCommit,
    openAttachment,
    home: source.home,
    contents: source.contents,
    device: GALLERY_HOST,
    attachment: galleryAttachment,
    open,
    showInFiles,
    commitHash: ({ commit }) => readCommitHash(sharedHistory.source, commit),
  }
}

/**
 * A reply that names the workspace's files: links, one at a line, links
 * whose text is code beside plain code, and paths written as code, of which
 * the workspace has some and not others.
 */
export const namedFilesMarkdown = `The cookie is renamed in [cookie.ts](src/auth/cookie.ts), and [session.ts](src/auth/session.ts#L5) reads it under the new name. The setup steps the [README](README.md#L42) lists still hold; from the home it is [~/Projects/shop/README.md](~/Projects/shop/README.md). The fix went in as [Fix the cart total when a coupon empties it](commit:${LINKED_COMMIT}), and the vendored parser's as [Bump the parser](commit:vendor/parser@${LINKED_COMMIT}), a commit of the repository in that folder.

在中文句子里，字符与图标之间同样留出间距：已经提交了[修正优惠券清空后的购物车合计](commit:${LINKED_COMMIT})，请查看。

A link's text written as code reads as the link's words: the page after the fix is [\`page-full.png\`](assets/page-full.png), the change [\`${LINKED_COMMIT}\`](commit:${LINKED_COMMIT}), the spec [\`RFC 6265\`](https://httpwg.org/specs/rfc6265.html), beside the plain code \`readSessionCookie\`.

Written as code, \`src/auth/cookie.ts\` and \`tests/login/auth.test.ts\` open, and \`README.md:42\` opens at its line; \`src/auth/token.ts\` is not in the workspace and stays code, and a bare name such as \`cookie\` is never asked about.`

/**
 * The files of `namedFilesMarkdown`'s inline code the workspace at `root`
 * has, as the backend keeps them with the reply once it ended.
 */
export function namedCodeFiles(root: string): CodeFile[] {
  return [
    { code: 'src/auth/cookie.ts', path: `${root}/src/auth/cookie.ts`, line: null },
    { code: 'tests/login/auth.test.ts', path: `${root}/tests/login/auth.test.ts`, line: null },
    { code: 'README.md:42', path: `${root}/README.md`, line: 42 },
  ]
}
