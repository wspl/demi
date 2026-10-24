import type { Block } from '@demicodes/protocol'
import type { MessageRow, MessageSource, SearchRow, SearchSource } from '@demicodes/web-ui/search/search'
import { ago } from './time'

/** A conversation a specimen's search reads: its row, and the text of its messages, oldest first. */
export interface SearchableConversation extends Omit<SearchRow, 'match' | 'titleRanges'> {
  messages: { blockId: string; text: string }[]
}

/** How long the fixture takes to answer, so the window shows it searching. */
const ANSWER_MS = 250

/** How many characters of a long line come before its first match. */
const LEAD = 40

/** The most characters of a match's line, as the backend cuts it. */
const LINE_MAX = 160

/** The text the backend searches in `blocks`: the user's messages and the answers, with who wrote each and when. */
export function searchableMessages(blocks: readonly Block[]): SearchableMessage[] {
  return blocks.flatMap((block): SearchableMessage[] => {
    if (block.type === 'user') {
      const text = block.content.flatMap((part) => (part.type === 'text' ? [part.text] : [])).join('\n')
      return text.trim() ? [{ blockId: block.id, author: 'user', time: block.createdAt, text }] : []
    }
    if (block.type === 'text' && block.text.trim()) {
      return [{ blockId: block.id, author: 'agent', time: block.createdAt, text: block.text }]
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
          // The placeholder holds no words, which no query finds.
          const byTitle = row.title !== null && holds(row.title, words)
          const message = messages.findLast((candidate) => holds(candidate.text, words))
          if (!byTitle && !message) {
            return []
          }
          const match = message ? { blockId: message.blockId, ...matchLine(message.text, words) } : null
          const titleRanges = byTitle && row.title !== null ? occurrences(row.title, words) : []
          return [{ byTitle, row: { ...row, titleRanges, match } }]
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
      title: 'Draft the quarterly newsletter',
      archived: false,
      lastActiveAt: ago(5 * 60 * 60 * 1000),
      messages: [
        { blockId: 'u1', text: 'Draft the release note for the tier contract change.' },
      ],
    },
  ]
}

/** A message of the conversation a specimen searches in: who wrote it, when, and its text. */
export interface SearchableMessage {
  blockId: string
  author: MessageRow['author']
  time: string
  text: string
}

/** The most messages one search of a conversation lists, as the backend's. */
const MESSAGES_MAX = 50

/**
 * A search of one conversation's `messages`, oldest first, that answers as the
 * backend's does, a beat later: the messages that hold every word, newest
 * first, each with its line and marks, at most fifty of them.
 */
export function fixtureMessageSearch(messages: () => readonly SearchableMessage[]): MessageSource {
  return (query, signal) =>
    new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        const words = query.split(/\s+/).filter(Boolean)
        const found = messages()
          .filter((message) => holds(message.text, words))
          .toReversed()
          .map(({ text, ...message }) => ({ ...message, ...matchLine(text, words) }))
        resolve({ rows: found.slice(0, MESSAGES_MAX), limited: found.length > MESSAGES_MAX })
      }, ANSWER_MS)
      signal.addEventListener('abort', () => {
        clearTimeout(timer)
        reject(signal.reason)
      }, { once: true })
    })
}

/** A conversation of the search window's list made of `messages`, active when its last one was written. */
export function searchableConversation(conversationId: string, title: string, messages: SearchableMessage[]): SearchableConversation {
  return { conversationId, title, archived: false, lastActiveAt: messages.at(-1)?.time ?? ago(0), messages }
}

/** The messages of the conversation the search window's specimen opens in, oldest first. */
export function shownConversationMessages(): SearchableMessage[] {
  const day = 24 * 60 * 60 * 1000
  return [
    { blockId: 'm1', author: 'user', time: ago(3 * day), text: 'Which port does the dev server use?' },
    { blockId: 'm2', author: 'agent', time: ago(3 * day - 60_000), text: 'It listens on port 4100, set as PORT in .env. The proxy in vite.config.ts forwards /api to it.' },
    { blockId: 'm3', author: 'user', time: ago(2 * day), text: 'The login page shows a blank screen after the redirect.' },
    { blockId: 'm4', author: 'agent', time: ago(2 * day - 60_000), text: 'The redirect drops the session cookie because its domain is example.com while the page runs on localhost. Setting the domain from the request fixes it.' },
    { blockId: 'm5', author: 'user', time: ago(60 * 60 * 1000), text: 'Change the port to 4200, something else holds 4100 now.' },
    { blockId: 'm6', author: 'agent', time: ago(59 * 60 * 1000), text: 'Done: PORT is 4200 in .env and in the proxy, and the server starts on it.' },
  ]
}

/** A conversation whose every answer retried something: more matches for retry than a search lists. */
export function retryingConversationMessages(): SearchableMessage[] {
  return Array.from({ length: 64 }, (_, index) => ({
    blockId: `r${index}`,
    author: index % 2 === 0 ? 'user' as const : 'agent' as const,
    time: ago((64 - index) * 15 * 60 * 1000),
    text: index % 2 === 0 ? `Retry the flaky upload test, attempt ${index / 2 + 1}.` : `The upload test passed after a retry of the request at attempt ${(index + 1) / 2}.`,
  }))
}
