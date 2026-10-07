import { Lexer, type Token } from 'marked'

/** Tokens that stand on lines of their own, which a line of plain text sets apart with a space. */
const BLOCKS = new Set(['paragraph', 'heading', 'code', 'blockquote', 'list', 'list_item', 'table', 'hr', 'space', 'html'])

/**
 * An agent's message as the words it shows, on one line, without its Markdown
 * syntax: for a place that says what a message holds in plain text, such as a
 * notification (`product.md` § Notifications). HTML shows nothing, as the page
 * does not render it.
 */
export function markdownPlainText(markdown: string): string {
  return wordsOf(Lexer.lex(markdown, { gfm: true })).replace(/\s+/g, ' ').trim()
}

function wordsOf(tokens: readonly Token[]): string {
  return tokens.map((token) => {
    const words = token.type === 'html'
      ? ''
      : token.type === 'list'
        ? token.items.map((item: Token) => wordsOf([item])).join(' ')
        : token.type === 'table'
          ? [token.header, ...token.rows].flat().map((cell: { tokens: Token[] }) => wordsOf(cell.tokens)).join(' ')
          : 'tokens' in token && token.tokens
            ? wordsOf(token.tokens)
            : 'text' in token ? String(token.text) : ''
    return BLOCKS.has(token.type) ? `${words} ` : words
  }).join('')
}
