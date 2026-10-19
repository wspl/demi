import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A message the backend refused shows Not Delivered under it with the
// cause, and, for a failure inside the backend, the request ID the user
// quotes (`turns.md` § Requesting). A Vue app on a happy-dom page; a few ms
// besides the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, h, nextTick } = await import('vue')
const { default: AgentMessageVirtualBlock } = await import('../blocks/AgentMessageVirtualBlock.vue')
type PendingSubmissionState = import('../types').PendingSubmissionState

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

async function row(submission: PendingSubmissionState): Promise<HTMLElement> {
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({
    render: () => h(AgentMessageVirtualBlock, {
      block: { type: 'delivery_failure', id: `delivery-failure:${submission.id}`, submission },
      isThinkingStreaming: false,
    }),
  })
  app.mount(root)
  unmount = () => app.unmount()
  await nextTick()
  return root
}

const message = { id: 'm1', way: 'send' as const, text: 'Look', attachments: [] }

test('a message refused for a failure inside the backend says so with the request ID to quote', async () => {
  const root = await row({ ...message, error: 'The message couldn’t be sent.', requestId: 'req_ffb05w4w6kxpdtz0' })
  const text = root.querySelector('[data-delivery-failure]')!.textContent!
  expect(text).toContain('The message couldn’t be sent.')
  expect(text).toContain('req_ffb05w4w6kxpdtz0')
})

test('a message refused for a cause of its own says the cause and names no request', async () => {
  const root = await row({ ...message, error: 'The device is offline.', requestId: null })
  const text = root.querySelector('[data-delivery-failure]')!.textContent!
  expect(text).toContain('The device is offline.')
  expect(text).not.toContain('req_')
})
