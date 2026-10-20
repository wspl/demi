import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// How a message from another agent shows in the view of the agent that
// receives it (`receivedAsBubble`). One the backend placed outside a run,
// from above or beside the receiver, shows as a bubble with its words, as a
// brief does, its sender named above it but for the main agent; one that is a
// step of a run, from below, keeps its row. A Vue app on a happy-dom page; a
// few ms besides the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick } = await import('vue')
const { default: AgentMessageVirtualBlock } = await import('../blocks/AgentMessageVirtualBlock.vue')
const { provideOriginRevealer } = await import('../agent-receipts')
const { provideBlobUrl } = await import('../media-source')
const { provideMediaViewer } = await import('../../files/media-viewer')
const { createdAt, model } = await import('./agent-harness')
type Block = import('@demicodes/protocol').Block
type ReceiptOrigin = import('../agent-receipts').ReceiptOrigin

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

const MAIN = { id: 'root', number: 0, description: 'root session' }
const REVIEW = { id: 'review', number: 2, description: 'Review the notes' }
const LINKS = { id: 'links', number: 3, description: 'Check the links' }

/**
 * The message `content` from `sender`, sent by its call `origin`, as a step of
 * its own run when `step`.
 */
function message(sender: typeof MAIN, content: string, origin: string, step = false): Block {
  const id = `${sender.id}-${origin}`
  return {
    type: 'agent_message', id, turnId: `${id}-turn`, createdAt, model, ...(step ? { run: id } : {}),
    message: { id, sender: { ...sender, round: 1 }, recipientId: 'notes', timestamp: createdAt, content, event: { type: 'message' }, origin },
  }
}

/** Shows `blocks`; answers where each sender's name led. */
async function show(blocks: Block[]): Promise<ReceiptOrigin[]> {
  const opened: ReceiptOrigin[] = []
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp(defineComponent({
    setup: () => {
      provideBlobUrl((source) => `blob:${source}`)
      provideMediaViewer()
      provideOriginRevealer((origin) => () => opened.push(origin))
      return () => blocks.map((block) => h(AgentMessageVirtualBlock, { key: block.id, block, isThinkingStreaming: false }))
    },
  }))
  app.mount(host)
  unmount = () => app.unmount()
  await nextTick()
  return opened
}

function text(): string {
  return document.body.textContent ?? ''
}

test('messages that ended a run show as bubbles, the sibling\'s named, and one that is a step as a row', async () => {
  const opened = await show([
    message(MAIN, 'Also keep the old heading.', 'main-send'),
    message(REVIEW, 'The table on line 40 is broken.', 'review-send'),
    message(LINKS, 'Seven of the links resolve so far.', 'links-send', true),
  ])
  const names = [...document.querySelectorAll<HTMLButtonElement>('button.notice-title-text')].map((button) => button.textContent?.trim())
  document.querySelector<HTMLButtonElement>('button.notice-title-text')!.click()

  // The main agent's and the sibling's words show; the child's stay behind
  // its row, *Message from agent: Check the links*.
  expect([text().includes('Also keep the old heading.'), text().includes('The table on line 40 is broken.')]).toEqual([true, true])
  expect(text()).not.toContain('Message from main agent')
  expect(text()).not.toContain('Seven of the links resolve so far.')
  expect(names).toEqual(['Review the notes', 'Check the links'])
  expect(opened).toEqual([{ node: 'review', blockId: 'review-send' }])
})
