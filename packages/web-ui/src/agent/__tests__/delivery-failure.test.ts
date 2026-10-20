import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A message the backend refused shows Not Delivered under it with the
// cause, and, for a failure inside the backend, the request ID the user
// quotes; the failure carries no button, and the message's own hover actions
// offer Copy, Edit and Delete, and the dock Retry while sending again can
// deliver one. A Vue app on a happy-dom page; a
// few ms besides the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, h, nextTick } = await import('vue')
const { default: AgentMessageVirtualBlock } = await import('../blocks/AgentMessageVirtualBlock.vue')
const { default: UndeliveredRetryChip } = await import('../UndeliveredRetryChip.vue')
const { provideBlobUrl } = await import('../media-source')
type PendingSubmissionState = import('../types').PendingSubmissionState
type MessageListBlock = import('../pending-steers').MessageListBlock

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

/** What the rows asked for, in order: Edit or Delete by the message's id, or the dock's Retry. */
let asked: string[] = []

async function mount(render: () => ReturnType<typeof h>): Promise<HTMLElement> {
  asked = []
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({
    setup() {
      provideBlobUrl((source) => `blob:${source}`)
      return render
    },
  })
  app.mount(root)
  unmount = () => app.unmount()
  await nextTick()
  return root
}

function row(block: MessageListBlock): Promise<HTMLElement> {
  return mount(() => h(AgentMessageVirtualBlock, {
    block,
    isThinkingStreaming: false,
    onEditSubmission: (id: string) => asked.push(`edit ${id}`),
    onDeleteSubmission: (id: string) => asked.push(`delete ${id}`),
  }))
}

const failure = (submission: PendingSubmissionState) =>
  row({ type: 'delivery_failure', id: `delivery-failure:${submission.id}`, submission })

const message = { id: 'm1', way: 'send' as const, text: 'Look', attachments: [] }

test('a message refused for a failure inside the backend says so with the request ID to quote', async () => {
  const root = await failure({ ...message, error: 'The message couldn’t be sent.', requestId: 'req_ffb05w4w6kxpdtz0' })
  const text = root.querySelector('[data-delivery-failure]')!.textContent!
  expect(text).toContain('The message couldn’t be sent.')
  expect(text).toContain('req_ffb05w4w6kxpdtz0')
})

test('a message refused for a cause of its own says the cause and names no request', async () => {
  const root = await failure({ ...message, error: 'The device is offline.', requestId: null })
  const text = root.querySelector('[data-delivery-failure]')!.textContent!
  expect(text).toContain('The device is offline.')
  expect(text).not.toContain('req_')
})

test('the failure under a message not delivered carries no button', async () => {
  const root = await failure({ ...message, error: 'The device is offline.' })
  expect(root.querySelectorAll('[data-delivery-failure] [role="button"], [data-delivery-failure] button')).toHaveLength(0)
})

/** The names of the message's hover actions, in the order it offers them. */
function hoverActions(root: HTMLElement): string[] {
  return [...root.querySelectorAll<HTMLElement>('.user-actions [aria-label]')].map((each) => each.getAttribute('aria-label')!)
}

function hoverAction(root: HTMLElement, name: string): HTMLElement {
  return root.querySelector<HTMLElement>(`.user-actions [aria-label="${name}"]`)!
}

test('a message not delivered offers Copy, Edit and Delete on hover, and Delete takes it away at once', async () => {
  const submission = { ...message, error: 'The device is offline.' }
  const root = await row({ type: 'pending_submission', id: 'pending-submission:m1', submission, startsTurn: false })
  expect(hoverActions(root)).toEqual(['Copy', 'Edit', 'Delete'])
  hoverAction(root, 'Edit').click()
  hoverAction(root, 'Delete').click()
  await nextTick()
  expect(asked).toEqual(['edit m1', 'delete m1'])
  expect(document.querySelector('[role="dialog"]')).toBeNull()
})

test('a message still on its way offers Copy alone', async () => {
  const submission = { ...message, error: null }
  const root = await row({ type: 'pending_submission', id: 'pending-submission:m1', submission, startsTurn: false })
  expect(hoverActions(root)).toEqual(['Copy'])
})

/** The dock's Retry, as the session shows it for its messages on their way. */
function dock(submissions: PendingSubmissionState[]): Promise<HTMLElement> {
  return mount(() => h(UndeliveredRetryChip, { pendingSubmissions: submissions, onRetry: () => asked.push('retry') }))
}

test('the dock offers Retry while a message not delivered can be sent again, and none while only one a resend cannot deliver is left', async () => {
  const gone = { ...message, id: 'm2', error: 'A file of this message is no longer available. Attach it again.', retryable: false as const }
  let root = await dock([{ ...message, error: 'The device is offline.' }, gone])
  const retry = root.querySelector<HTMLElement>('[data-retry-undelivered]')!
  expect(retry.textContent!.trim()).toBe('Retry')
  retry.click()
  expect(asked).toEqual(['retry'])
  unmount?.()
  root = await dock([gone, { ...message, id: 'm3', error: null }])
  expect(root.querySelector('[data-retry-undelivered]')).toBeNull()
})
