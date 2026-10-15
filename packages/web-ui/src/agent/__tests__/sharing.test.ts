import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, test } from 'bun:test'
import { effectScope, type EffectScope } from 'vue'

// The share dialog's flow (`sharing.md` § Making, updating and removing a
// share): Share opens the dialog at once and makes the link in it, copying
// it from inside the click; a share that exists opens with its link; and
// what the dialog's buttons do. A page of happy-dom with a clipboard
// stand-in that, like Safari, takes a write only while the click runs; a
// few milliseconds.

GlobalRegistrator.register({ url: 'https://demi.test/' })
// After the page, since the clipboard library looks for a browser as it loads.
const { useShare } = await import('../sharing')
const { toasts, dismissToast } = await import('../../infra/toast')
import type { ExistingShare, ShareService, ShareTarget } from '../sharing'

const answer: ShareTarget = { kind: 'answer', blockId: 'text-1' }
let copied: string[] = []
let calls: string[] = []
let failWith: Error | null = null
let held: ExistingShare[] = []
/** The click being handled: a write outside it is refused, as Safari refuses one. */
let inClick = false
/** The answer of the next `create`, which the test gives when it wants. */
let pending: { resolve: () => void; reject: (error: Error) => void } | null = null
let scope: EffectScope

const service: ShareService = {
  link: (token) => `https://demi.test/s/${token}`,
  create: (target) => {
    calls.push(`create ${target.kind}`)
    return new Promise((resolve, reject) => {
      pending = { resolve: () => resolve({ token: 'new', url: 'https://demi.test/s/new' }), reject }
    })
  },
  update: async (token) => {
    calls.push(`update ${token}`)
    if (failWith) throw failWith
    return { token, url: `https://demi.test/s/${token}` }
  },
  remove: async (token) => {
    calls.push(`remove ${token}`)
    if (failWith) throw failWith
  },
}

beforeEach(() => {
  copied = []
  calls = []
  failWith = null
  held = []
  pending = null
  for (const toast of [...toasts]) dismissToast(toast.id)
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: {
      write: async (items: ClipboardItem[]) => {
        if (!inClick) throw new DOMException('The request is not allowed by the user agent', 'NotAllowedError')
        copied.push(await (await items[0]!.getType('text/plain')).text())
      },
    },
  })
  scope = effectScope()
})

afterEach(() => scope.stop())

const make = () => scope.run(() => useShare(() => service, () => held))!

/** Runs `handler` as the handler of a click. */
function click(handler: () => unknown): void {
  inClick = true
  try {
    void handler()
  } finally {
    inClick = false
  }
}

/** Lets the promises that have settled run their callbacks. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

test('Share opens the dialog at once on Creating link…, then shows the link, already copied', async () => {
  const share = make()
  click(() => share.start(answer))
  expect(share.isOpen.value).toBe(true)
  expect(share.shown.value).toEqual({ target: answer, status: 'creating' })
  expect(calls).toEqual(['create answer'])
  expect(copied).toEqual([])
  pending!.resolve()
  await settle()
  expect(share.shown.value).toEqual({ target: answer, status: 'ready', token: 'new', url: 'https://demi.test/s/new' })
  expect(copied).toEqual(['https://demi.test/s/new'])
  expect(share.copied.value).toBe(true)
  expect(toasts).toEqual([])
})

test('a share that exists opens with its link, with no new share and nothing copied', async () => {
  held = [{ kind: 'conversation', token: 'old' }, { kind: 'answer', token: 'mine', blockId: 'text-1' }]
  const share = make()
  click(() => share.start(answer))
  expect(calls).toEqual([])
  expect(copied).toEqual([])
  expect(share.shown.value).toEqual({ target: answer, status: 'ready', token: 'mine', url: 'https://demi.test/s/mine' })
  click(() => share.start({ kind: 'conversation' }))
  expect(share.shown.value).toMatchObject({ status: 'ready', url: 'https://demi.test/s/old' })
  // Another answer's share is not this answer's.
  click(() => share.start({ kind: 'answer', blockId: 'text-2' }))
  expect(calls).toEqual(['create answer'])
})

test('a refused share says why in the dialog, copies nothing, and Try Again makes it', async () => {
  const share = make()
  click(() => share.start(answer))
  pending!.reject(Object.assign(new Error('refused'), { code: 'share_target_invalid', params: {} }))
  await settle()
  expect(share.isOpen.value).toBe(true)
  expect(share.shown.value).toMatchObject({ target: answer, status: 'failed' })
  expect(copied).toEqual([])
  // The dialog says it: no toast, neither of the share nor of a copy.
  expect(toasts).toEqual([])
  click(() => share.retry())
  expect(share.shown.value).toEqual({ target: answer, status: 'creating' })
  pending!.resolve()
  await settle()
  expect(share.shown.value).toMatchObject({ status: 'ready', url: 'https://demi.test/s/new' })
  expect(copied).toEqual(['https://demi.test/s/new'])
  expect(calls).toEqual(['create answer', 'create answer'])
})

test('a link made after the dialog moved to another share does not replace it', async () => {
  held = [{ kind: 'conversation', token: 'old' }]
  const share = make()
  click(() => share.start(answer))
  click(() => share.start({ kind: 'conversation' }))
  pending!.resolve()
  await settle()
  expect(share.shown.value).toMatchObject({ target: { kind: 'conversation' }, status: 'ready', url: 'https://demi.test/s/old' })
})

test('Update Link shows its progress in place of the link, moves it under its token and says so; an answer has none', async () => {
  held = [{ kind: 'conversation', token: 'old' }]
  const share = make()
  click(() => share.start({ kind: 'conversation' }))
  const updating = share.update()
  expect(share.shown.value).toMatchObject({ status: 'updating', token: 'old' })
  await updating
  expect(share.shown.value).toMatchObject({ status: 'ready', token: 'old', url: 'https://demi.test/s/old' })
  expect(calls).toEqual(['update old'])
  expect(toasts.map((toast) => toast.title)).toEqual(['Link Updated'])
  held = [{ kind: 'answer', token: 'mine', blockId: 'text-1' }]
  click(() => share.start(answer))
  await share.update()
  expect(calls).toEqual(['update old'])
})

test('Stop Sharing removes the share and closes the dialog; a failure leaves it open', async () => {
  held = [{ kind: 'conversation', token: 'old' }]
  const share = make()
  click(() => share.start({ kind: 'conversation' }))
  failWith = new Error('offline')
  await share.stop()
  expect(share.isOpen.value).toBe(true)
  expect(toasts.map((toast) => toast.title)).toEqual(['Could Not Stop Sharing'])
  failWith = null
  await share.stop()
  expect(calls).toEqual(['remove old', 'remove old'])
  expect(share.isOpen.value).toBe(false)
})
