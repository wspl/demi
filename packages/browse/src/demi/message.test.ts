// `demi.message` and `demi.turn` against a stand-in of the web app and its
// backend, in Playwright's own Chromium, which `bun browse` installs on its
// first call: about a second, the browser's start, and `demi.turn`'s looks
// at the backend, four a second. No cheaper test shows what the page shows,
// which is what the calls report and wait for. The stand-in speaks Japanese:
// the calls find the page's controls by the hooks the web app's components
// give them, never by their words.
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium, type Browser } from 'playwright'
import type { ConversationSummary } from '@demicodes/web/src/api/generated/web-api'
import { conversationSummary } from '@demicodes/web/src/__tests__/product-state'
import { slotPorts, type Slot } from '../slot'
import { Failure } from '../tool'
import { messageOn } from './message'
import { turnOn } from './turn'

const UNDELIVERED = '00000000-0000-4000-8000-000000000001'
const FAILING = '00000000-0000-4000-8000-000000000002'
const HELPING = '00000000-0000-4000-8000-000000000003'
const ON_ITS_WAY = '00000000-0000-4000-8000-000000000004'
const ATTACHING = '00000000-0000-4000-8000-000000000005'
const DRAFTED = '00000000-0000-4000-8000-000000000006'

/** What the backend holds: a conversation is there once its message was delivered. */
const conversations: ConversationSummary[] = [
  conversationSummary(HELPING, 'Helping', { revision: 1, lastTurn: { id: 'block-1', outcome: 'finished', answerStart: 'Hi' } }),
  conversationSummary(ON_ITS_WAY, 'On its way', { revision: 1, lastTurn: { id: 'block-1', outcome: 'finished', answerStart: 'Hi' } }),
]

/**
 * The page of a conversation: a composer whose Enter sends, the Requesting
 * row and the composer's Stop from the send until the backend answers, and
 * the notice the send answers with.
 */
const PAGE = `<!doctype html>
<div><textarea data-composer aria-label="メッセージ"></textarea></div>
<div id="flow"></div>
<script>
  const composer = document.querySelector('textarea')
  composer.addEventListener('keydown', async (event) => {
    if (event.key !== 'Enter') return
    event.preventDefault()
    const flow = document.getElementById('flow')
    const stop = document.createElement('button')
    stop.setAttribute('aria-label', '停止')
    stop.setAttribute('data-composer-stop', '')
    composer.after(stop)
    flow.innerHTML = '<div data-transcript-wait="requesting"><span>リクエスト中</span></div>'
    const answer = await fetch(location.pathname + '/send', { method: 'POST', body: composer.value })
    flow.innerHTML = await answer.text()
    stop.remove()
  })
</script>`

/**
 * The page of a conversation whose composer carries files as the product's
 * does: each file the composer's file input takes is a capsule in the
 * message's text, and Enter sends the text with the capsules' files. `draft`
 * is what the composer holds when the page opens.
 */
function composerPage(draft: string): string {
  return `<!doctype html>
<div><input type="file" multiple hidden>
<div><div contenteditable="true" role="textbox" data-composer aria-label="メッセージ">${draft}</div></div></div>
<script>
  const input = document.querySelector('input')
  const composer = document.querySelector('[role=textbox]')
  input.addEventListener('change', () => {
    for (const file of input.files) {
      composer.insertAdjacentHTML('beforeend', '<span class="message-capsule" contenteditable="false">' + file.name + '</span>')
    }
  })
  composer.addEventListener('keydown', (event) => {
    if (event.key !== 'Enter') return
    event.preventDefault()
    const text = [...composer.childNodes].filter((node) => node.nodeType === Node.TEXT_NODE).map((node) => node.textContent).join('')
    const files = [...composer.querySelectorAll('.message-capsule')].map((capsule) => capsule.textContent)
    fetch(location.pathname + '/send', { method: 'POST', body: JSON.stringify({ text, files }) })
  })
</script>`
}

/** What the composer pages sent, by their conversation. */
const sentMessages = new Map<string, unknown>()
let messageSent = () => {}
const sentMessage = new Promise<void>((resolve) => {
  messageSent = resolve
})

/** Delivers the message sent in ON_ITS_WAY, which the backend holds until then. */
let deliver = () => {}
const delivery = new Promise<void>((resolve) => {
  deliver = resolve
})

/** Counts the backend's answers about the conversations once the message on its way was sent. */
let looks = 0
let lookedThrice = () => {}
const thirdLook = new Promise<void>((resolve) => {
  lookedThrice = resolve
})
let sent = false

/** A notice as the page's `ErrorNotice` draws it: its sentence, the reason, and its controls. */
function notice(label: string, detail: string, action: string): string {
  return `<div role="alert"><p>${label}</p><p>${detail}</p><button>${action}</button><button aria-label="Copy"></button></div>`
}

const server = Bun.serve({
  port: 0,
  hostname: '127.0.0.1',
  async fetch(request) {
    const path = new URL(request.url).pathname
    if (path === '/api/conversations') {
      if (sent && ++looks === 3) {
        lookedThrice()
      }
      return Response.json({ conversations })
    }
    if (path === `/chat/${ON_ITS_WAY}/send`) {
      sent = true
      await delivery
      // The message's turn ran and ended by the time its delivery reached the page.
      conversations[1] = conversationSummary(ON_ITS_WAY, 'On its way', {
        revision: 2,
        lastTurn: { id: 'block-2', outcome: 'finished', answerStart: 'Done' },
      })
      return new Response('')
    }
    if (path === `/chat/${UNDELIVERED}/send`) {
      // The backend refused the creation: it holds no conversation, and the page says so.
      return new Response(`<div data-delivery-failure>${notice('このメッセージは送信されませんでした。', 'That device is the conversation’s primary host', '再試行')}</div>`)
    }
    if (path === `/chat/${FAILING}/send`) {
      conversations.push(conversationSummary(FAILING, 'Failing', {
        revision: 1,
        lastTurn: { id: 'block-1', outcome: 'failed', answerStart: null },
      }))
      return new Response(notice('プロバイダがエラーを返しました。', 'Rate limit exceeded', 'レポートをコピー'))
    }
    if (path === `/chat/${ATTACHING}` || path === `/chat/${DRAFTED}`) {
      const draft = path.endsWith(DRAFTED) ? 'Look at <span class="message-capsule" contenteditable="false">chart.png</span>' : ''
      return new Response(composerPage(draft), { headers: { 'content-type': 'text/html' } })
    }
    if (path.endsWith('/send') && (path.includes(ATTACHING) || path.includes(DRAFTED))) {
      sentMessages.set(path.split('/')[2]!, await request.json())
      messageSent()
      return new Response('')
    }
    if (path === `/chat/${HELPING}/send`) {
      // The message's turn ended, and a subagent it started still runs.
      conversations[0] = conversationSummary(HELPING, 'Helping', {
        status: 'running',
        revision: 2,
        lastTurn: { id: 'block-2', outcome: 'finished', answerStart: 'A subagent looks into it' },
      })
      return new Response('')
    }
    return new Response(PAGE, { headers: { 'content-type': 'text/html' } })
  },
})

const slot: Slot = { root: '/', number: 0, ports: { ...slotPorts(0), network: server.port! }, folder: '/' }
let browser: Browser | null = null
const folder = mkdtempSync(join(tmpdir(), 'browse-message-'))

beforeAll(async () => {
  if (existsSync(chromium.executablePath())) {
    browser = await chromium.launch()
  }
})

afterAll(async () => {
  await browser?.close()
  await server.stop(true)
  rmSync(folder, { recursive: true, force: true })
})

test.skipIf(!existsSync(chromium.executablePath()))(
  'a message carries the files it names, and one written over a draft that holds files fails before it deletes them',
  async () => {
    const page = await browser!.newPage()
    const picture = join(folder, 'white.png')
    writeFileSync(picture, 'not really a picture')
    await page.goto(`http://127.0.0.1:${server.port}/chat/${ATTACHING}`)
    await messageOn(slot, page, 'What is in this picture?', { files: [picture], wait: false })
    await sentMessage
    expect(sentMessages.get(ATTACHING)).toEqual({ text: 'What is in this picture?', files: ['white.png'] })

    await page.goto(`http://127.0.0.1:${server.port}/chat/${DRAFTED}`)
    const failure = await messageOn(slot, page, 'What is in this chart?', { wait: false })
      .then(() => 'answered', (error: unknown) => error instanceof Failure ? error.message : String(error))
    expect(failure).toBe(
      'The composer\'s draft holds chart.png, which writing the message in its place would delete; '
      + 'remove the files first, or attach them with demi.message(text, { files })',
    )
    expect(sentMessages.has(DRAFTED)).toBe(false)
  },
)

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
      `Conversation ${UNDELIVERED}: このメッセージは送信されませんでした。 That device is the conversation’s primary host`,
    )
    expect(await failure(FAILING)).toBe(
      `The turn of the message failed in conversation ${FAILING}: プロバイダがエラーを返しました。 Rate limit exceeded`,
    )
  },
)

test.skipIf(!existsSync(chromium.executablePath()))(
  'a turn waits for the message on its way while the page shows Requesting, though the backend is idle',
  async () => {
    const page = await browser!.newPage()
    await page.goto(`http://127.0.0.1:${server.port}/chat/${ON_ITS_WAY}`)
    await messageOn(slot, page, 'Run the tests', { wait: false })
    const turn = turnOn(slot, page)
    // Before the message is delivered, the backend still reports the
    // conversation idle after its earlier turn.
    expect(await Promise.race([turn.then(() => 'ended'), thirdLook.then(() => 'waiting')])).toBe('waiting')
    deliver()
    expect(await turn).toEqual({ status: 'idle', revision: 2, lastTurn: { id: 'block-2', outcome: 'finished' } })
  },
)

test.skipIf(!existsSync(chromium.executablePath()))(
  'a message ends with its turn, while the subagents the turn started still run',
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
