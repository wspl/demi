import { Marked, type MarkedExtension } from 'marked'
import markedCjkFriendly from 'marked-cjk-friendly'

/**
 * A Markdown reader of the page's: GitHub Flavored Markdown whose emphasis
 * beside CJK text reads as a CJK writer means it (`product.md` § Writing a
 * message), so `**注意：**这是` is bold where plain CommonMark reads the
 * stars beside CJK punctuation as text. Every reader on the page starts
 * here and adds its own `extensions` after it, so none can miss either.
 */
export function markdownReader(...extensions: MarkedExtension[]): Marked {
  return new Marked({ gfm: true }, markedCjkFriendly(), ...extensions)
}
