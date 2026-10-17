import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// Add Model is a search: choosing a match adds the model at once, its id
// made free with a suffix when the list has it, and opens it in Edit Model,
// where an id another model of the list has is refused before anything is
// sent and a free one is saved (`models.md` § Adding a model from
// models.dev). The Settings page of
// web-ui on a page of happy-dom, its host's save and search stubbed. Cost:
// the search's 150 ms pause, and tens of milliseconds besides.

GlobalRegistrator.register({ url: 'http://127.0.0.1:3271/settings' })

/** The phone switch (`phone.ts`): the desktop's unless a test turns it, and `turn` tells the page. */
class Width extends EventTarget {
  matches = false
  turn(phone: boolean): void {
    this.matches = phone
    this.dispatchEvent(new Event('change'))
  }
}
const width = new Width()
Object.defineProperty(window, 'matchMedia', { value: () => width, configurable: true })
// After the page, since the components ask for its elements.
await import('@demicodes/web-ui/__tests__/vue-components')
const { createApp, h, shallowRef } = await import('vue')
const { default: SettingsProvidersPage } = await import('@demicodes/web-ui/settings/SettingsProvidersPage.vue')
const { createOverlayStore } = await import('@demicodes/web-ui/overlay/overlayStore')
type SettingsProviderEntry = import('@demicodes/web-ui/settings/types').SettingsProviderEntry
type SettingsModelDraft = import('@demicodes/web-ui/settings/types').SettingsModelDraft

afterEach(() => {
  document.body.innerHTML = ''
})

const model = (id: string) => ({
  id, name: id, contextWindow: 128_000, outputLimit: null, efforts: [], extensions: [], fastTier: null, enabled: true,
})

/** A compatible API whose list has the match's id already. */
const proxy: SettingsProviderEntry = {
  id: 'proxy', name: 'Proxy', kind: 'api_key', vendorId: null, compatible: true,
  baseUrl: 'https://proxy.example/v1', wireApi: 'openai-chat', apiKey: '', keyConfigured: false, configured: true,
  modelSource: 'manual', catalogFetched: null, state: 'ready', enabled: true,
  models: [model('gemini-3.8-flash'), model('other')], accounts: [], logo: null,
}

const flash: SettingsModelDraft = {
  id: 'gemini-3.8-flash', name: 'Gemini 3.8 Flash', contextWindow: 1_000_000, outputLimit: 65_536,
  efforts: ['low', 'high'], extensions: ['.png'], fastTier: null,
}

/** Waits for `ready` to hold, turning the event loop and the timers; fails after two seconds. */
async function until(ready: () => boolean, what: string): Promise<void> {
  const deadline = Date.now() + 2000
  while (!ready()) {
    if (Date.now() > deadline) {
      throw new Error(`${what} did not happen`)
    }
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
}

function type(input: HTMLInputElement, text: string): void {
  input.focus()
  input.value = text
  input.dispatchEvent(new Event('input', { bubbles: true }))
}

const input = (label: string) => document.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)
/** A control the page shows as a button, by its text or its name. */
const button = (name: string) =>
  [...document.querySelectorAll<HTMLElement>('button, [role="button"]')].find(
    (candidate) => candidate.textContent?.trim() === name || candidate.getAttribute('aria-label') === name,
  )
/** Whether the button is disabled, as a native one or as one that says so. */
const disabled = (control: HTMLElement) =>
  control.getAttribute('aria-disabled') === 'true' || (control as HTMLButtonElement).disabled === true

test('a match chosen in Add Model is added with a free id and opens in Edit Model, which refuses a taken id and saves a free one', async () => {
  const saved: { draft: SettingsModelDraft; original: string | null }[] = []
  const providers = shallowRef([proxy])
  const element = document.createElement('div')
  document.body.append(element)
  const app = createApp({
    render: () =>
      h(SettingsProvidersPage, {
        providers: providers.value,
        vendors: [],
        overlayStore: createOverlayStore(),
        selectedId: 'proxy',
        detailOpen: true,
        saveModel: async (provider: SettingsProviderEntry, draft: SettingsModelDraft, original: { id: string } | null) => {
          saved.push({ draft, original: original?.id ?? null })
          providers.value = [{ ...provider, models: [...provider.models, { ...draft, enabled: true }] }]
        },
        searchModels: async (_provider: string, query: string) =>
          query.includes('gemini') ? [{ ref: 'google/gemini-3.8-flash', vendor: 'google', model: flash }] : [],
      }),
  })
  app.mount(element)
  try {
    await until(() => button('Add Model') !== undefined, 'the page shows Add Model')
    button('Add Model')!.click()
    await until(() => document.querySelector('[role="dialog"] input') !== null, 'Add Model opens')
    const search = document.querySelector<HTMLInputElement>('[role="dialog"] input')!
    type(search, 'gemini')
    const row = () =>
      [...document.querySelectorAll<HTMLElement>('[role^="menuitem"], [role="option"]')].find((option) =>
        option.textContent?.includes('Gemini 3.8 Flash'),
      )
    await until(() => row() !== undefined, 'the match is offered')
    row()!.click()

    await until(() => input('Model ID') !== null, 'Edit Model opens')
    expect(saved).toEqual([{ draft: { ...flash, id: 'gemini-3.8-flash-2' }, original: null }])
    expect(input('Model ID')!.value).toBe('gemini-3.8-flash-2')
    expect(disabled(button('Save')!)).toBe(false)

    type(input('Model ID')!, 'other')
    await until(() => document.body.textContent!.includes('Another model of this provider has this ID'), 'the id is refused')
    expect(disabled(button('Save')!)).toBe(true)
    expect(saved).toHaveLength(1)

    // A free id saves the model under it, and the dialog goes.
    type(input('Model ID')!, 'mine')
    await until(() => !disabled(button('Save')!), 'Save takes the free id')
    button('Save')!.click()
    await until(() => input('Model ID') === null, 'Edit Model closes')
    expect(saved[1]).toEqual({ draft: { ...flash, id: 'mine' }, original: 'gemini-3.8-flash-2' })
  } finally {
    app.unmount()
  }
})

/** The page over `providers`, `proxy` selected, its host's saves and changes recorded. */
function mountPage(providers: { value: SettingsProviderEntry[] }, hooks: {
  saved?: { draft: SettingsModelDraft; original: string | null }[]
  changed?: Partial<SettingsProviderEntry>[]
}) {
  const element = document.createElement('div')
  document.body.append(element)
  const app = createApp({
    render: () =>
      h(SettingsProvidersPage, {
        providers: providers.value,
        vendors: [],
        overlayStore: createOverlayStore(),
        selectedId: 'proxy',
        detailOpen: true,
        saveModel: async (provider: SettingsProviderEntry, draft: SettingsModelDraft, original: { id: string } | null) => {
          hooks.saved?.push({ draft, original: original?.id ?? null })
          const models = original
            ? provider.models.map((entry) => (entry.id === original.id ? { ...draft, enabled: entry.enabled } : entry))
            : [...provider.models, { ...draft, enabled: true }]
          providers.value = [{ ...provider, models }]
        },
        searchModels: async () => [],
        onChange: (_provider: SettingsProviderEntry, changes: Partial<SettingsProviderEntry>) => {
          hooks.changed?.push(changes)
        },
      }),
  })
  app.mount(element)
  return app
}

// On a phone a list row hides no Remove at its end, where a tap meant to open
// it lands; an API key's own page removes it from its bottom, asking first as
// the desktop's row does, and the page then goes back to the list
// (`web-application.md` § The phone layout). The desktop keeps the row's
// Remove and no Remove Provider on the page. Tens of milliseconds.
test('on a phone an API key is removed from the bottom of its page, asked first, and the page goes back to the list', async () => {
  const providers = shallowRef([proxy, { ...proxy, id: 'other', name: 'Other' }])
  const removed: string[] = []
  const detailOpen = shallowRef(true)
  const selectedId = shallowRef('proxy')
  const element = document.createElement('div')
  document.body.append(element)
  const app = createApp({
    render: () =>
      h(SettingsProvidersPage, {
        providers: providers.value,
        vendors: [],
        overlayStore: createOverlayStore(),
        selectedId: selectedId.value,
        detailOpen: detailOpen.value,
        saveModel: async () => {},
        'onUpdate:detailOpen': (open: boolean) => {
          detailOpen.value = open
        },
        onRemove: async (id: string) => {
          removed.push(id)
          // The host selects the next provider at once, as the gallery's does.
          providers.value = providers.value.filter((provider) => provider.id !== id)
          selectedId.value = providers.value[0]!.id
        },
      }),
  })
  /** The Remove at the end of a list row, which the desktop shows on hover. */
  const rowRemove = () => document.querySelector('[aria-pressed] [aria-label="Remove"]')
  width.turn(true)
  app.mount(element)
  try {
    await until(() => button('Remove Provider…') !== undefined, 'the page shows Remove Provider')
    expect(rowRemove()).toBeNull()

    button('Remove Provider…')!.click()
    await until(() => document.body.textContent!.includes('Remove “Proxy”?'), 'the removal is asked')
    expect(removed).toEqual([])
    // The question's own button says Remove; the model rows' Remove is an icon.
    const confirm = () =>
      [...document.querySelectorAll<HTMLElement>('[role="button"]')].find((candidate) => candidate.textContent?.trim() === 'Remove')
    await until(() => confirm() !== undefined, 'the question offers Remove')
    confirm()!.click()
    await until(() => removed.length === 1 && !detailOpen.value, 'the provider goes and the list shows')
    expect(removed).toEqual(['proxy'])

    width.turn(false)
    await until(() => rowRemove() !== null, 'the desktop row offers Remove')
    expect(button('Remove Provider…')).toBeUndefined()
  } finally {
    width.turn(false)
    app.unmount()
  }
})

// Custom adds a model whose id and name are the text typed, its window
// unknown, and Edit Model saves it so (`models.md` § Model fields). Cost:
// the search's 150 ms pause, and tens of milliseconds besides.
test('Custom adds the typed model at once without a context window, and Edit Model saves it as it is', async () => {
  const saved: { draft: SettingsModelDraft; original: string | null }[] = []
  const providers = shallowRef([{ ...proxy, models: [] }])
  const app = mountPage(providers, { saved })
  try {
    await until(() => button('Add Model') !== undefined, 'the page shows Add Model')
    button('Add Model')!.click()
    await until(() => document.querySelector('[role="dialog"] input') !== null, 'Add Model opens')
    type(document.querySelector<HTMLInputElement>('[role="dialog"] input')!, 'my-model')
    const custom = () =>
      [...document.querySelectorAll<HTMLElement>('[role^="menuitem"], [role="option"]')].find((option) =>
        option.textContent?.includes('Custom'),
      )
    await until(() => custom() !== undefined, 'Custom is offered')
    custom()!.click()

    await until(() => input('Model ID') !== null, 'Edit Model opens')
    const added: SettingsModelDraft = {
      id: 'my-model', name: 'my-model', contextWindow: null, outputLimit: null, efforts: [], extensions: [], fastTier: null,
    }
    expect(saved).toEqual([{ draft: added, original: null }])
    type(input('Name')!, 'My Model')
    await until(() => !disabled(button('Save')!), 'Save takes a model without a window')
    button('Save')!.click()
    await until(() => input('Model ID') === null, 'Edit Model closes')
    expect(saved[1]).toEqual({ draft: { ...added, name: 'My Model' }, original: 'my-model' })
  } finally {
    app.unmount()
  }
})

// Emptying the saved key or endpoint and leaving the field removes it, as
// the API's null does; leaving the key's field as it was keeps the key
// (`models.md` § Adding a model from models.dev). Tens of milliseconds.
test('a saved key or endpoint emptied and left is removed, and a key field left as it was keeps the key', async () => {
  const changed: Partial<SettingsProviderEntry>[] = []
  const providers = shallowRef([{ ...proxy, keyConfigured: true }])
  const app = mountPage(providers, { changed })
  try {
    await until(() => input('API key') !== null, 'the page shows the key')
    const key = input('API key')!
    key.focus()
    key.dispatchEvent(new FocusEvent('focus'))
    key.blur()
    key.dispatchEvent(new FocusEvent('blur'))
    expect(changed).toEqual([])

    type(key, '')
    key.dispatchEvent(new FocusEvent('blur'))
    const endpoint = input('Endpoint')!
    type(endpoint, '')
    endpoint.dispatchEvent(new FocusEvent('blur'))
    await until(() => changed.length === 2, 'both removals are sent')
    expect(changed).toEqual([{ apiKey: '' }, { baseUrl: '' }])
  } finally {
    app.unmount()
  }
})

/** A subscription of `accounts`, selected. */
const subscription = (accounts: string[]): SettingsProviderEntry => ({
  id: 'claude', name: 'Claude Code', kind: 'subscription', vendorId: null, compatible: false,
  baseUrl: '', wireApi: 'anthropic-messages', apiKey: '', configured: true,
  modelSource: 'catalog', catalogFetched: null, state: 'ready', enabled: true, models: [],
  accounts: accounts.map((label, index) => ({ id: `a${index}`, label, plan: '', active: index === 0, quota: [] })),
  rotation: 'manual', logo: null,
})

// Adding a second account and choosing In Order or Balanced first warn
// that several personal subscriptions may be against the vendor's terms;
// Continue goes on and is told to the host, Cancel drops the step, and a
// user who went on is never asked again (`providers.md` § Account
// rotation). Tens of milliseconds.
test('the warning about several accounts asks before the second account and a rotation, and once told never again', async () => {
  const events: string[] = []
  const warned = shallowRef(false)
  const providers = shallowRef([subscription([])])
  const element = document.createElement('div')
  document.body.append(element)
  const overlayStore = createOverlayStore()
  const app = createApp({
    render: () =>
      h(SettingsProvidersPage, {
        providers: providers.value,
        vendors: [],
        overlayStore,
        selectedId: 'claude',
        detailOpen: true,
        multiAccountWarned: warned.value,
        saveModel: async () => {},
        onSignIn: () => events.push('signIn'),
        onChange: (_provider: SettingsProviderEntry, changes: Partial<SettingsProviderEntry>) =>
          events.push(`rotation:${changes.rotation}`),
        onMultiAccountWarned: () => {
          events.push('warned')
          warned.value = true
        },
      }),
  })
  app.mount(element)
  const warning = () => document.body.textContent?.includes('may be against the vendor’s terms') ?? false
  try {
    // The first account asks nothing.
    await until(() => button('Add Account') !== undefined, 'the page shows Add Account')
    button('Add Account')!.click()
    expect(events).toEqual(['signIn'])

    // The second does; Cancel adds nothing.
    providers.value = [subscription(['alex@example.com'])]
    await until(() => document.body.textContent!.includes('alex@example.com'), 'the first account shows')
    button('Add Account')!.click()
    await until(warning, 'the warning shows')
    button('Cancel')!.click()
    await until(() => !warning(), 'Cancel closes the warning')
    expect(events).toEqual(['signIn'])

    // With two accounts the rotation is offered; choosing Balanced asks too.
    providers.value = [subscription(['alex@example.com', 'sam@example.com'])]
    await until(() => button('Rotation') !== undefined, 'the rotation shows')

    button('Rotation')!.click()
    const choice = () =>
      [...document.querySelectorAll<HTMLElement>('[role^="menuitem"]')].find((row) => row.textContent?.includes('Balanced'))
    await until(() => choice() !== undefined, 'the rotation menu opens')
    choice()!.click()
    await until(warning, 'the warning shows for a rotation')
    button('Continue')!.click()
    await until(() => events.length === 3, 'Continue goes on')
    expect(events).toEqual(['signIn', 'warned', 'rotation:balanced'])

    // Told once: another account goes on without asking.
    await until(() => !warning(), 'the warning closes')
    button('Add Account')!.click()
    expect(warning()).toBe(false)
    expect(events).toEqual(['signIn', 'warned', 'rotation:balanced', 'signIn'])
  } finally {
    app.unmount()
  }
})

// A provider that makes images has the Image Generation switch, which
// tells the host the new value; one that makes none has no switch
// (`providers.md` § Provider management). Tens of milliseconds.
test('the Image Generation switch shows only for a provider that makes images and tells the host its new value', async () => {
  const changes: Partial<SettingsProviderEntry>[] = []
  const providers = shallowRef<SettingsProviderEntry[]>([{ ...subscription([]), imageGeneration: true }])
  const element = document.createElement('div')
  document.body.append(element)
  const app = createApp({
    render: () =>
      h(SettingsProvidersPage, {
        providers: providers.value,
        vendors: [],
        overlayStore: createOverlayStore(),
        selectedId: 'claude',
        detailOpen: true,
        saveModel: async () => {},
        onChange: (_provider: SettingsProviderEntry, change: Partial<SettingsProviderEntry>) => changes.push(change),
      }),
  })
  app.mount(element)
  const imageSwitch = () =>
    document.querySelector<HTMLElement>('[role="switch"][aria-label="Image Generation"]') ?? undefined
  try {
    await until(() => imageSwitch() !== undefined, 'the switch shows')
    expect(imageSwitch()!.getAttribute('aria-checked')).toBe('true')
    imageSwitch()!.click()
    await until(() => changes.length === 1, 'the change is told')
    expect(changes).toEqual([{ imageGeneration: false }])

    providers.value = [subscription([])]
    await until(() => imageSwitch() === undefined, 'the switch goes')
  } finally {
    app.unmount()
  }
})
