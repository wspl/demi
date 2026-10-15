import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, test } from 'bun:test'
import { effectScope, type EffectScope } from 'vue'

// The share dialog's flow (`sharing.md` § Making, updating and removing a
// share): what Share does for a conversation that has no share and for one
// that has, and what the dialog's buttons do. A page of happy-dom with a
// clipboard stand-in; a few milliseconds.

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
let scope: EffectScope

const service: ShareService = {
  link: (token) => `https://demi.test/s/${token}`,
  create: async (target) => {
    calls.push(`create ${target.kind}`)
    if (failWith) throw failWith
    return { token: 'new', url: 'https://demi.test/s/new' }
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
  for (const toast of [...toasts]) dismissToast(toast.id)
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: {
      write: async (items: ClipboardItem[]) => {
        copied.push(await (await items[0]!.getType('text/plain')).text())
      },
    },
  })
  scope = effectScope()
})

afterEach(() => scope.stop())

const make = () => scope.run(() => useShare(() => service, () => held))!

test('Share makes the share, copies its link and opens the dialog on it', async () => {
  const share = make()
  await share.start(answer)
  expect(calls).toEqual(['create answer'])
  expect(copied).toEqual(['https://demi.test/s/new'])
  expect(share.isOpen.value).toBe(true)
  expect(share.shown.value).toEqual({ target: answer, token: 'new', url: 'https://demi.test/s/new' })
  expect(share.copied.value).toBe(true)
})

test('a share that exists opens with its link, with no new share and nothing copied', async () => {
  held = [{ kind: 'conversation', token: 'old' }, { kind: 'answer', token: 'mine', blockId: 'text-1' }]
  const share = make()
  await share.start(answer)
  expect(calls).toEqual([])
  expect(copied).toEqual([])
  expect(share.shown.value?.url).toBe('https://demi.test/s/mine')
  await share.start({ kind: 'conversation' })
  expect(share.shown.value?.url).toBe('https://demi.test/s/old')
  // Another answer's share is not this answer's.
  await share.start({ kind: 'answer', blockId: 'text-2' })
  expect(calls).toEqual(['create answer'])
})

test('a refused share is a toast with the backend’s words and opens nothing', async () => {
  failWith = Object.assign(new Error('refused'), { code: 'share_target_invalid', params: {} })
  const share = make()
  await share.start(answer)
  expect(share.isOpen.value).toBe(false)
  expect(toasts.map((toast) => [toast.title, toast.message])).toEqual([
    ['Could Not Share Answer', 'Only the text that ends a reply can be shared.'],
  ])
})

test('Update Link takes the conversation again under its token and says so; an answer has none', async () => {
  held = [{ kind: 'conversation', token: 'old' }]
  const share = make()
  await share.start({ kind: 'conversation' })
  await share.update()
  expect(calls).toEqual(['update old'])
  expect(toasts.map((toast) => toast.title)).toEqual(['Link Updated'])
  held = [{ kind: 'answer', token: 'mine', blockId: 'text-1' }]
  await share.start(answer)
  await share.update()
  expect(calls).toEqual(['update old'])
})

test('Stop Sharing removes the share and closes the dialog; a failure leaves it open', async () => {
  held = [{ kind: 'conversation', token: 'old' }]
  const share = make()
  await share.start({ kind: 'conversation' })
  failWith = new Error('offline')
  await share.stop()
  expect(share.isOpen.value).toBe(true)
  expect(toasts.map((toast) => toast.title)).toEqual(['Could Not Stop Sharing'])
  failWith = null
  await share.stop()
  expect(calls).toEqual(['remove old', 'remove old'])
  expect(share.isOpen.value).toBe(false)
})
