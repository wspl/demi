import { afterAll, beforeAll, expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium, type Browser, type BrowserContext, type Page } from 'playwright'
import { claimSchema, createConversationSchema, deviceAnswerSchema, devicesSchema, modelCatalogSchema } from '@demicodes/web/src/api/generated/web-api'
import { previewLab, type CaseResult } from './run'
import { ACCOUNT, freePort, startStack, type Stack } from './stack'

// The web preview's behavior laboratory as a scenario suite (`preview.md`
// § Tests, `scenarios.md` § Browser suite): the spike's lab cases, each run
// on the lab page loaded directly and in a tab of the user's browser in the
// product, through the relay, the conversation's `preview` stream and the
// Host's engine on a paired runner. It needs Chrome, which `DEMI_TEST_CHROME`
// names, as the browser suite does; an ordinary run skips it, and
// `DEMI_TEST_PREVIEW_LAB_CASES` picks some cases. No model answers: the
// conversation is made through the web API, with the development backend's
// echo model. The whole lab takes
// about 15 minutes: each of its 245 cases loads the lab twice.

const chrome = process.env.DEMI_TEST_CHROME
/** Parts of the ids of the cases to run, comma-separated, to run some of them; all of them without it. */
const cases = process.env.DEMI_TEST_PREVIEW_LAB_CASES?.split(',').filter(Boolean) ?? []
/** How long the whole lab may take. */
const LAB_MS = 60 * 60 * 1000
/**
 * The cases whose baseline this machine cannot give: every site of the lab is
 * on loopback, and this one needs a public site to compare with.
 */
const UNREPRODUCIBLE = ['local-network-from-public-page']

let root: string
let stack: Stack | undefined
let browser: Browser | undefined

beforeAll(async () => {
  if (!chrome) {
    return
  }
  root = await mkdtemp(join(tmpdir(), 'preview-lab-'))
  stack = await startStack(root)
  browser = await chromium.launch({ executablePath: chrome })
}, 10 * 60 * 1000)

afterAll(async () => {
  await browser?.close()
  await stack?.stop()
  if (root) {
    await rm(root, { recursive: true, force: true })
  }
})

/**
 * The product's page, signed in, on a conversation on the runner's device
 * with the echo model, which no message started, its panel open.
 */
/** The runner's device, once a test claimed it. */
let claimed: Promise<{ id: string }> | undefined

async function conversationPage(): Promise<{ context: BrowserContext; page: Page }> {
  const context = await browser!.newContext({ baseURL: stack!.web })
  const page = await context.newPage()
  const signedIn = await page.request.post('/api/auth/login', { data: ACCOUNT })
  expect(signedIn.status()).toBe(200)
  // The runner's device, claimed once as the page claims one, and online.
  claimed ??= page.request
    .post('/api/devices/claim', { data: claimSchema.parse({ code: stack!.runner.code }) })
    .then(async (answer) => deviceAnswerSchema.parse(await answer.json()).device)
  const device = await claimed
  for (;;) {
    const { devices } = devicesSchema.parse(await (await page.request.get('/api/devices')).json())
    if (devices.some((each) => each.id === device.id && each.state === 'online')) {
      break
    }
    await Bun.sleep(100)
  }
  // A conversation on the device with the echo model, which no message started.
  const catalog = modelCatalogSchema.parse(await (await page.request.get('/api/models')).json())
  const echo = catalog.providers.find((provider) => provider.models.some((model) => model.id === 'echo'))!
  const id = crypto.randomUUID()
  const conversation = createConversationSchema.parse({
    id,
    model: { providerId: echo.providerId, modelId: 'echo' },
    target: { kind: 'device', deviceId: device.id, path: stack!.runner.home },
  })
  expect((await page.request.post('/api/conversations', { data: conversation })).ok()).toBe(true)
  await page.goto(`/chat/${id}`)
  await page.getByRole('button', { name: 'Open panel' }).click()
  return { context, page }

}

test.skipIf(!chrome)('a tab of the user’s browser answers the lab’s cases as a direct load does, or as its stated deviation says', async () => {
  const { context, page } = await conversationPage()
  const counts = await previewLab(
    { page, context, report: join(root, 'preview-lab.json'), filter: cases, ports: { app: freePort(), aux: freePort(), secondAux: freePort() } },
    (line) => console.log(line),
  )
  console.log(JSON.stringify(counts))
  const rows: CaseResult[] = await Bun.file(join(root, 'preview-lab.json')).json()
  const unexpected = rows.filter((row) => row.status === 'fail' || (row.status === 'invalid-baseline' && !UNREPRODUCIBLE.includes(row.id)))
  expect(unexpected.map((row) => `${row.id}: ${row.error ?? ''}`)).toEqual([])
}, LAB_MS)

/**
 * A tab of the user's browser in Mobile is a phone (`preview.md` § Mobile):
 * to the site, by its user agent and client hints, and to the page's first
 * script, by its navigator; its frame is a phone's size. About 10 s: the
 * product's page, and one page loaded twice.
 */
test.skipIf(!chrome)('a tab of the user’s browser in Mobile is a phone to the site and to the page’s first script', async () => {
  const site = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch(request) {
      const headers = Object.fromEntries(['user-agent', 'sec-ch-ua-mobile', 'sec-ch-ua-platform'].map((name) => [name, request.headers.get(name)]))
      return new Response(`<!doctype html><title>Device</title><script>
window.firstScript = {
  userAgent: navigator.userAgent,
  platform: navigator.platform,
  maxTouchPoints: navigator.maxTouchPoints,
  mobile: navigator.userAgentData?.mobile,
  hintedPlatform: navigator.userAgentData?.platform,
  width: innerWidth,
};
</script><pre id="headers">${JSON.stringify(headers)}</pre>`, { headers: { 'content-type': 'text/html' } })
    },
  })
  try {
    const { page } = await conversationPage()
    await page.getByRole('button', { name: 'New Tab' }).click()
    const address = page.getByRole('textbox', { name: 'Browser address' })
    await address.fill(`http://localhost:${site.port}/`)
    await address.press('Enter')
    const frame = page.frameLocator('iframe[title="Page"]:visible')
    await frame.locator('#headers').waitFor({ timeout: 60_000 })
    await page.getByRole('button', { name: 'Viewport' }).click()
    await page.getByRole('menuitem', { name: 'Mobile' }).click()
    await frame.locator('#headers', { hasText: 'Android' }).waitFor({ timeout: 30_000 })
    const headers = JSON.parse(await frame.locator('#headers').innerText())
    expect(headers['sec-ch-ua-mobile']).toBe('?1')
    expect(headers['sec-ch-ua-platform']).toBe('"Android"')
    expect(headers['user-agent']).toMatch(/^Mozilla\/5\.0 \(Linux; Android 10; K\) .* Mobile Safari\/537\.36$/)
    const element = await page.locator('iframe[title="Page"]:visible').elementHandle()
    const first = await (await element!.contentFrame())!.evaluate(() => Reflect.get(window, 'firstScript'))
    expect(first).toEqual({
      userAgent: headers['user-agent'],
      platform: 'Linux armv81',
      maxTouchPoints: 5,
      mobile: true,
      hintedPlatform: 'Android',
      width: 390,
    })
  } finally {
    await site.stop(true)
  }
}, 120_000)
