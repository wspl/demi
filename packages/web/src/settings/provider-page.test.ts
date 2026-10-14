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
// After the page, since the components ask for its elements.
await import('../__tests__/vue-components')
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
