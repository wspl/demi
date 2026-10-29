import { Marked, type MarkedExtension } from 'marked'
import markedCjkFriendly from 'marked-cjk-friendly'

/**
 * A Markdown reader of the page's: GitHub Flavored Markdown whose emphasis
 * beside CJK text reads as a CJK writer means it, so `**注意：**这是` is bold where
 * plain CommonMark reads the stars beside CJK punctuation as text, and whose
 * strikethrough takes two tildes. Every reader on the page starts here and adds
 * its own `extensions` after it, so none can miss any of them.
 */
export function markdownReader(...extensions: MarkedExtension[]): Marked {
  return new Marked({ gfm: true }, markedCjkFriendly(), twoTildeStrikethrough, ...extensions)
}

/**
 * Strikethrough is `~~gone~~` alone, as markdown-it reads it. GFM also takes a
 * single tilde, which a reply uses for a range, as in `18:00~19:00` or `30~86`:
 * two ranges in a paragraph struck everything between them.
 */
const twoTildeStrikethrough: MarkedExtension = {
  tokenizer: {
    // False hands a pair of tildes to the reader's own rule; nothing makes a
    // single tilde text.
    del(src) {
      return src.startsWith('~~') ? false : undefined
    },
  },
}
