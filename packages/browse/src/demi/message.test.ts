// `demi.message` against a stand-in of the web app and its backend, in
// Playwright's own Chromium, which `bun browse` installs on its first call:
// about a second, the browser's start. No cheaper test shows what the page
// shows, which is what the call reports.
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { chromium, type Browser } from 'playwright'
import type { ConversationSummary } from '@demicodes/web/src/api/generated/web-api'
import { conversationSummary } from '@demicodes/web/src/__tests__/product-state'
import { slotPorts, type Slot } from '../slot'
import { Failure } from '../tool'
import { messageOn } from './message'

const UNDELIVERED = '00000000-0000-4000-8000-000000000001'
const FAILING = '00000000-0000-4000-8000-000000000002'
const HELPING = '00000000-0000-4000-8000-000000000003'

/** What the backend holds: a conversation is there once its message was delivered. */
const conversations: ConversationSummary[] = [
  conversationSummary(HELPING, 'Helping', { revision: 1, lastTurn: { id: 'block-1', outcome: 'finished', answerStart: 'Hi' } }),
]

/** The page of a conversation: a composer whose Enter sends, and the notice the send answers with. */
const PAGE = `<!doctype html>
<textarea aria-label="Message"></textarea>
<div id="flow"></div>
<script>
  const composer = document.querySelector('textarea')
  composer.addEventListener('keydown', async (event) => {
    if (event.key !== 'Enter') return
    event.preventDefault()
    const answer = await fetch(location.pathname + '/send', { method: 'POST', body: composer.value })
    document.getElementById('flow').innerHTML = await answer.text()
  })
</script>`

/** A notice as the page's `ErrorNotice` draws it: its sentence, the reason, and its controls. */
function notice(label: string, detail: string, action: string): string {
  return `<div role="alert"><p>${label}</p><p>${detail}</p><button>${action}</button><button aria-label="Copy"></button></div>`
}

const server = Bun.serve({
  port: 0,
  hostname: '127.0.0.1',
  fetch(request) {
    const path = new URL(request.url).pathname
    if (path === '/api/conversations') {
      return Response.json({ conversations })
    }
    if (path === `/chat/${UNDELIVERED}/send`) {
      // The backend refused the creation: it holds no conversation, and the page says so.
      return new Response(notice('This message was not delivered.', 'That device is the conversation’s primary host', 'Retry'))
    }
    if (path === `/chat/${FAILING}/send`) {
      conversations.push(conversationSummary(FAILING, 'Failing', {
        revision: 1,
        lastTurn: { id: 'block-1', outcome: 'failed', answerStart: null },
      }))
      return new Response(notice('The provider returned an error.', 'Rate limit exceeded', 'Copy Report'))
    }
    if (path === `/chat/${HELPING}/send`) {
      // The message's turn ended, and a helper agent it started still runs.
      conversations[0] = conversationSummary(HELPING, 'Helping', {
        status: 'running',
        revision: 2,
        lastTurn: { id: 'block-2', outcome: 'finished', answerStart: 'A helper looks into it' },
      })
      return new Response('')
    }
    return new Response(PAGE, { headers: { 'content-type': 'text/html' } })
  },
})

const slot: Slot = { root: '/', number: 0, ports: { ...slotPorts(0), network: server.port! }, folder: '/' }
let browser: Browser | null = null

beforeAll(async () => {
  if (existsSync(chromium.executablePath())) {
    browser = await chromium.launch()
  }
})

afterAll(async () => {
  await browser?.close()
  await server.stop(true)
})

test.skipIf(!existsSync(chromium.executablePath()))(
  'a message that is not delivered, or whose turn fails, fails the call at once with what the page says',
  async () => {
    const page = await browser!.newPage()
    const failure = (id: string) =>
      page.goto(`http://127.0.0.1:${server.port}/chat/${id}`)
        .then(() => messageOn(slot, page, 'Hello'))
        .then(() => 'answered', (error: unknown) => error instanceof Failure ? error.message : String(error))
    // Either would wait the call's ten minutes for a turn, past the test's limit.
    expect(await failure(UNDELIVERED)).toBe(
      `Conversation ${UNDELIVERED}: This message was not delivered. That device is the conversation’s primary host`,
    )
    expect(await failure(FAILING)).toBe(
      `The turn of the message failed in conversation ${FAILING}: The provider returned an error. Rate limit exceeded`,
    )
  },
)

test.skipIf(!existsSync(chromium.executablePath()))(
  'a message ends with its turn, while the helper agents the turn started still run',
  async () => {
    const page = await browser!.newPage()
    await page.goto(`http://127.0.0.1:${server.port}/chat/${HELPING}`)
    // Waiting for the helpers would run into this timeout.
    const end = await messageOn(slot, page, 'Ask a helper', { timeout: 3000 })
    expect(end).toEqual({
      conversation: HELPING,
      turn: { status: 'running', revision: 2, lastTurn: { id: 'block-2', outcome: 'finished' } },
    })
  },
)
