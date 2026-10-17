import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// Settings › Agent's Image Generation rows (`images.md` § Which entry
// makes an image): a row for each vendor the backend names, showing the
// entry Automatic takes beside it, and a choice of the vendor's entries
// told to the host as the `imageEntries` it saves. The Settings page of
// web-ui on a page of happy-dom. Tens of milliseconds.

GlobalRegistrator.register({ url: 'http://127.0.0.1:3271/settings/agent' })
await import('@demicodes/web-ui/__tests__/vue-components')
const { createApp, h, ref } = await import('vue')
const { default: SettingsAgent } = await import('@demicodes/web-ui/settings/SettingsAgent.vue')
const { imageVendorRows } = await import('@demicodes/web-ui/settings/image-entries')
const { createOverlayStore } = await import('@demicodes/web-ui/overlay/overlayStore')
type SettingsProviderEntry = import('@demicodes/web-ui/settings/types').SettingsProviderEntry
type ImageEntryChoices = import('@demicodes/web-ui/settings/types').ImageEntryChoices

afterEach(() => {
  document.body.innerHTML = ''
})

async function until(ready: () => boolean, what: string): Promise<void> {
  const deadline = Date.now() + 2000
  while (!ready()) {
    if (Date.now() > deadline) {
      throw new Error(`${what} did not happen`)
    }
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
}

function entry(partial: Partial<SettingsProviderEntry> & Pick<SettingsProviderEntry, 'id' | 'name' | 'kind'>): SettingsProviderEntry {
  return {
    vendorId: null, compatible: false, baseUrl: '', wireApi: 'openai-chat', apiKey: '', modelSource: 'catalog',
    catalogFetched: null, state: 'ready', enabled: true, models: [], accounts: [], logo: null, ...partial,
  }
}

const chatgpt = entry({
  id: 'codex', name: 'ChatGPT', kind: 'subscription', imageVendor: 'openai', imageGeneration: true,
  accounts: [{ id: 'a1', label: 'alex@example.com', plan: 'Plus', active: true, signedOut: false, quota: [] }],
})
const key = entry({ id: 'openai', name: 'OpenAI', kind: 'api_key', imageVendor: 'openai', imageGeneration: true })
const grok = entry({ id: 'grok', name: 'Grok', kind: 'subscription', imageVendor: 'xai', imageGeneration: false })

// Planted defects: the row without Automatic's entry beside it; a choice
// not told to the host, or told without the other vendors' choices; an
// entry whose Image Generation is off offered in the menu.
test('an Image Generation row shows the entry Automatic takes and tells the host the entry chosen', async () => {
  const chosen = ref<ImageEntryChoices>({ xai: 'grok' })
  const told: ImageEntryChoices[] = []
  const element = document.createElement('div')
  document.body.append(element)
  const app = createApp({
    render: () =>
      h(SettingsAgent, {
        overlayStore: createOverlayStore(),
        providers: [],
        models: {},
        catalogModels: {},
        automaticModel: null,
        imageVendors: imageVendorRows([chatgpt, key, grok], { openai: 'codex', xai: null }),
        smallModel: null,
        sameProvider: false,
        sendWhileRunning: 'steer',
        resumeAfterRestart: true,
        resumeAtRetry: false,
        imageEntries: chosen.value,
        'onUpdate:imageEntries': (next: ImageEntryChoices) => {
          told.push(next)
          chosen.value = next
        },
      }),
  })
  app.mount(element)
  const trigger = (vendor: string) => document.querySelector<HTMLElement>(`[aria-label="${vendor}"]`) ?? undefined
  const items = () => [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')]
  try {
    await until(() => trigger('OpenAI') !== undefined, 'the rows show')
    expect(trigger('OpenAI')!.textContent?.trim()).toBe('Automatic (ChatGPT · alex@example.com)')
    // xAI's only entry is off: its row is Automatic alone, whatever was chosen.
    expect(trigger('xAI')!.textContent?.trim()).toBe('Automatic')

    trigger('OpenAI')!.click()
    await until(() => items().length > 0, 'the menu opens')
    expect(items().map((item) => item.textContent?.trim())).toEqual([
      'Automatic (ChatGPT · alex@example.com)', 'ChatGPT · alex@example.com', 'OpenAI',
    ])
    items()[2]!.click()
    await until(() => told.length === 1, 'the choice is told')
    expect(told).toEqual([{ xai: 'grok', openai: 'openai' }])
    await until(() => trigger('OpenAI')!.textContent?.trim() === 'OpenAI', 'the choice shows')

    trigger('OpenAI')!.click()
    await until(() => items().length > 0, 'the menu opens again')
    items()[0]!.click()
    await until(() => told.length === 2, 'Automatic is told')
    expect(told[1]).toEqual({ xai: 'grok' })
  } finally {
    app.unmount()
  }
})
