import type { Block } from '@demicodes/protocol'
import type { SearchRow, SearchSource } from '@demicodes/web-ui/search/search'
import { ago } from './time'

/** A conversation a specimen's search reads: its row, and the text of its messages, oldest first. */
export interface SearchableConversation extends Omit<SearchRow, 'match'> {
  messages: { blockId: string; text: string }[]
}

/** How long the fixture takes to answer, so the window shows it searching. */
const ANSWER_MS = 250

/** How many characters of a long line come before its first match. */
const LEAD = 40

/** The most characters of a match's line, as the backend cuts it. */
const LINE_MAX = 160

/** The text the backend searches in `blocks`: the user's messages and the answers. */
export function searchableMessages(blocks: readonly Block[]): SearchableConversation['messages'] {
  return blocks.flatMap((block) => {
    if (block.type === 'user') {
      const text = block.content.flatMap((part) => (part.type === 'text' ? [part.text] : [])).join('\n')
      return text.trim() ? [{ blockId: block.id, text }] : []
    }
    if (block.type === 'text' && block.text.trim()) {
      return [{ blockId: block.id, text: block.text }]
    }
    return []
  })
}

/** Where each word occurs in `text`, case ignored, as `[start, end]` UTF-16 offsets, in order, overlaps joined. */
function occurrences(text: string, words: readonly string[]): [number, number][] {
  const lower = text.toLowerCase()
  const found: [number, number][] = []
  for (const word of words) {
    const wanted = word.toLowerCase()
    for (let at = lower.indexOf(wanted); at >= 0; at = lower.indexOf(wanted, at + 1)) {
      found.push([at, at + wanted.length])
    }
  }
  found.sort((a, b) => a[0] - b[0])
  const joined: [number, number][] = []
  for (const [start, end] of found) {
    const last = joined.at(-1)
    if (last && start <= last[1]) {
      last[1] = Math.max(last[1], end)
    } else {
      joined.push([start, end])
    }
  }
  return joined
}

/** The line around the first match, cut to the backend's length, and the words' places in it. */
function matchLine(text: string, words: readonly string[]): { text: string; ranges: number[][] } {
  const found = occurrences(text, words)
  const first = found[0]?.[0] ?? 0
  let lineStart = text.lastIndexOf('\n', first - 1) + 1
  let lineEnd = text.indexOf('\n', first)
  if (lineEnd < 0) {
    lineEnd = text.length
  }
  while (lineStart < lineEnd && /\s/.test(text[lineStart]!)) lineStart++
  while (lineEnd > lineStart && /\s/.test(text[lineEnd - 1]!)) lineEnd--
  let start = lineStart
  let end = lineEnd
  if (end - start > LINE_MAX) {
    start = Math.max(lineStart, first - LEAD)
    end = Math.min(lineEnd, start + LINE_MAX)
    start = Math.max(lineStart, end - LINE_MAX)
  }
  const cutStart = start > lineStart
  const cutEnd = end < lineEnd
  if (cutStart) start++
  if (cutEnd) end--
  const shift = cutStart ? 1 - start : -start
  return {
    text: `${cutStart ? '…' : ''}${text.slice(start, end)}${cutEnd ? '…' : ''}`,
    ranges: found
      .filter(([from, to]) => from < end && to > start)
      .map(([from, to]) => [Math.max(from, start) + shift, Math.min(to, end) + shift]),
  }
}

/** Whether `text` holds every word, case ignored. */
function holds(text: string, words: readonly string[]): boolean {
  const lower = text.toLowerCase()
  return words.every((word) => lower.includes(word.toLowerCase()))
}

/**
 * A search over `conversations` that answers as the backend's does, a beat
 * later: titles that hold every word first, then conversations with a
 * message that does, each group most recently active first, the newest such
 * message with its line and marks.
 */
export function fixtureSearch(conversations: () => readonly SearchableConversation[]): SearchSource {
  return (query, signal) =>
    new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        const words = query.split(/\s+/).filter(Boolean)
        const found = conversations().flatMap(({ messages, ...row }) => {
          const byTitle = holds(row.title, words)
          const message = messages.findLast((candidate) => holds(candidate.text, words))
          if (!byTitle && !message) {
            return []
          }
          const match = message ? { blockId: message.blockId, ...matchLine(message.text, words) } : null
          return [{ byTitle, row: { ...row, match } }]
        })
        found.sort((a, b) => Number(b.byTitle) - Number(a.byTitle) || b.row.lastActiveAt.localeCompare(a.row.lastActiveAt))
        resolve(found.slice(0, 50).map(({ row }) => row))
      }, ANSWER_MS)
      signal.addEventListener('abort', () => {
        clearTimeout(timer)
        reject(signal.reason)
      }, { once: true })
    })
}

/** The conversations of the search window's specimen, one of them archived. */
export function searchConversations(): SearchableConversation[] {
  return [
    {
      conversationId: 'search-ts2307',
      title: 'TS2307 after the package split',
      archived: false,
      lastActiveAt: ago(3 * 60 * 1000),
      messages: [
        { blockId: 'u1', text: 'The build fails with TS2307: Cannot find module @demicodes/utils.' },
        { blockId: 't1', text: 'The package moved, so its path mapping in tsconfig.json still points at src/utils. Point it at packages/utils/src and TS2307 goes away.' },
      ],
    },
    {
      conversationId: 'search-login',
      title: 'Login test after the session cookie rename',
      archived: false,
      lastActiveAt: ago(40 * 60 * 1000),
      messages: [
        { blockId: 'u1', text: 'The relogin test fails since the cookie rename.' },
        { blockId: 't1', text: 'The test still reads demi_session; the cookie is demi_sid now. Updating the fixture fixes the relogin flow.' },
      ],
    },
    {
      conversationId: 'search-paths',
      title: '模块路径配置',
      archived: true,
      lastActiveAt: ago(3 * 24 * 60 * 60 * 1000),
      messages: [
        { blockId: 'u1', text: '为什么 TS2307 一直出现？' },
        { blockId: 't1', text: '路径别名没有在 vite.config.ts 里配置，所以构建找不到模块。' },
      ],
    },
    {
      conversationId: 'search-wording',
      title: 'Release note wording for the tier contract',
      archived: false,
      lastActiveAt: ago(5 * 60 * 60 * 1000),
      messages: [
        { blockId: 'u1', text: 'Draft the release note for the tier contract change.' },
      ],
    },
  ]
}
