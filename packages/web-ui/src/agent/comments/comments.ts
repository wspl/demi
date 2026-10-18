import type { MessageComment } from '@demicodes/protocol'

/**
 * Comments on a reply's words (`comments-and-ask.md`): each saved comment
 * waits in the conversation's draft until a message sends it, and the reply
 * shows the marks of both, a sent one dimmer, read from the sent messages.
 */

/** A comment the transcript marks: one waiting in the draft, or one a message sent. */
export interface MarkedComment extends MessageComment {
  sent: boolean
}

/**
 * What the transcript's host does with the comments waiting in its draft;
 * absent where a transcript takes none, as a share's page or a subagent's.
 */
export interface ReplyCommenting {
  /** The comments waiting in the draft, in the order they were saved. */
  waiting: readonly MessageComment[]
  /** Saves a new comment, which waits from then on. */
  add: (comment: MessageComment) => void
  /** Changes the text of the waiting comment `id`. */
  edit: (id: string, text: string) => void
  /** Removes the waiting comment `id`. */
  remove: (id: string) => void
}

/** The comments a sent message holds, as its `user` or `steer` block keeps them; none for any other block. */
export function sentComments(block: { type: string; comments?: readonly MessageComment[] }): readonly MessageComment[] {
  return block.type === 'user' || block.type === 'steer' ? block.comments ?? [] : []
}

/**
 * `comments` as the model reads them before a message's text
 * (`comments-and-ask.md` § Sending them): each quoted text as a `>` quote,
 * its comment after it, a blank line between them; empty for none. The
 * page's twin of the backend's `comments_markdown`, which writes them into
 * the model's request.
 */
export function commentsMarkdown(comments: readonly Pick<MessageComment, 'quote' | 'text'>[]): string {
  return comments
    .map(({ quote, text }) => {
      const lines = quote.split('\n').map((line) => line.replace(/\r$/, ''))
      // As Rust's `str::lines`: a final line break ends the last line and starts none.
      if (lines.at(-1) === '') {
        lines.pop()
      }
      return `${lines.map((line) => (line === '' ? '>' : `> ${line}`)).join('\n')}\n\n${text}`
    })
    .join('\n\n')
}

/** `markdown`, a message's text, with its comments before it as the model reads them. */
export function withComments(markdown: string, comments: readonly Pick<MessageComment, 'quote' | 'text'>[]): string {
  const lead = commentsMarkdown(comments)
  return lead && markdown ? `${lead}\n\n${markdown}` : lead || markdown
}

/**
 * `comments` in the order the conversation reads: by the place of their
 * reply among `blockIds`, then by where each starts in it. A comment whose
 * reply the list does not hold goes last, in the order given.
 */
export function inReadingOrder<T extends MessageComment>(comments: readonly T[], blockIds: readonly string[]): T[] {
  const place = new Map(blockIds.map((id, index) => [id, index]))
  const at = (comment: T) => place.get(comment.blockId) ?? blockIds.length
  return [...comments].sort((a, b) => at(a) - at(b) || a.start - b.start)
}
