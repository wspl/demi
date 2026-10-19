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

/** What the row asked for, in order: Retry, Edit or Delete, by the message's id. */
let asked: string[] = []

async function row(submission: PendingSubmissionState): Promise<HTMLElement> {
  asked = []
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({
    render: () => h(AgentMessageVirtualBlock, {
      block: { type: 'delivery_failure', id: `delivery-failure:${submission.id}`, submission },
      isThinkingStreaming: false,
      onRetrySubmission: (id: string) => asked.push(`retry ${id}`),
      onEditSubmission: (id: string) => asked.push(`edit ${id}`),
      onDeleteSubmission: (id: string) => asked.push(`delete ${id}`),
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

/** The labels of the row's buttons, in the order it offers them. */
function rowButtons(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>('[data-delivery-failure] [role="button"]')]
}

// Retry, Edit and Delete, as Messages offers Try Again and Delete; a
// refusal sending again cannot change offers Edit first and no Retry
// (`turns.md` § Following the agent's work).
test('a message not delivered offers Retry, Edit and Delete, and one a resend cannot deliver offers Edit first and no Retry', async () => {
  let root = await row({ ...message, error: 'The device is offline.' })
  expect(rowButtons(root).map((button) => button.textContent!.trim())).toEqual(['Retry', 'Edit', 'Delete'])
  rowButtons(root)[0]!.click()
  rowButtons(root)[1]!.click()
  expect(asked).toEqual(['retry m1', 'edit m1'])
  unmount?.()
  root = await row({ ...message, error: 'A file of this message is no longer available. Attach it again.', retryable: false })
  expect(rowButtons(root).map((button) => button.textContent!.trim())).toEqual(['Edit', 'Delete'])
})

test('Delete on a message not delivered asks first, and Cancel keeps it', async () => {
  const root = await row({ ...message, error: 'The device is offline.' })
  const dialogButton = (label: string) => [...document.querySelectorAll<HTMLElement>('[role="dialog"] [role="button"], [role="dialog"] button')]
    .find((button) => button.textContent!.trim() === label)
  rowButtons(root).at(-1)!.click()
  await nextTick()
  expect(document.querySelector('[role="dialog"]')?.textContent).toContain('Delete This Message?')
  dialogButton('Cancel')!.click()
  await nextTick()
  expect(asked).toEqual([])
  rowButtons(root).at(-1)!.click()
  await nextTick()
  dialogButton('Delete')!.click()
  await nextTick()
  expect(asked).toEqual(['delete m1'])
})
