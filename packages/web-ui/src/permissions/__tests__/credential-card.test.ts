import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A credential request's card says how long the credential is kept, as the
// agent decided it, and offers no choice of it; Save answers the values
// alone. One Vue app on a happy-dom page; a few ms besides the page's
// start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, h, nextTick } = await import('vue')
const { default: CredentialRequestCard } = await import('../CredentialRequestCard.vue')
type Request = import('../types').CredentialRequestView
type Answer = import('../types').CredentialAnswer

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

function mount(request: Request, decided: unknown[][]): HTMLElement {
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({
    render: () =>
      h(CredentialRequestCard, {
        request,
        count: 1,
        onDecide: (id: string, decision: string, credential?: Answer) => decided.push([id, decision, credential]),
      }),
  })
  app.mount(root)
  unmount = () => app.unmount()
  return root
}

const deployToken: Request = {
  kind: 'credential',
  id: 'cr-deploy',
  name: 'GitHub token for deploys',
  reason: 'To deploy the site whenever you ask',
  fields: [{ name: 'token', label: 'Personal access token', kind: 'secret' }],
  keep: '30d',
  renewal: false,
}

test('the card says how long the agent keeps the credential, offers no choice, and saves the values alone', async () => {
  const decided: unknown[][] = []
  const root = mount(deployToken, decided)
  expect(root.textContent).toContain('Kept for 30 days')
  expect(root.querySelector('[role="radiogroup"], [role="radio"], select')).toBeNull()
  expect(root.querySelectorAll('input')).toHaveLength(1)

  const input = root.querySelector('input')!
  input.value = 'ghp_123'
  input.dispatchEvent(new Event('input'))
  await nextTick()
  const save = [...root.querySelectorAll<HTMLElement>('[role="button"]')].find((button) => button.textContent?.trim() === 'Save')!
  save.click()
  expect(decided).toEqual([['cr-deploy', 'allow', { values: { token: 'ghp_123' } }]])
})

test('a credential kept always says it stays until the user removes it', () => {
  const root = mount({ ...deployToken, keep: 'always' }, [])
  expect(root.textContent).toContain('Kept until you remove it')
})
