/**
 * The web preview's behavior laboratory (`preview.md` § Tests): each of the
 * spike's lab cases runs twice in the slot's Chromium, once on the lab page
 * loaded directly in a context of its own, its baseline, and once in a tab
 * of the user's browser in the product, through the relay, the
 * conversation's `preview` stream and the Host's engine. A case passes when
 * the preview answers as the direct load does, or as its stated deviation
 * says; one the preview declines by design is unsupported. It is a check
 * script's part, not an automated test: it needs the slot's backend, web
 * app and paired runner, and a conversation on that runner open with its
 * panel, as `check.ts` sets them up.
 */
import { writeFileSync } from 'node:fs'
import { deepStrictEqual } from 'node:assert'
import type { BrowserContext, Frame, Page } from 'playwright'
import { startLab } from './server'

export type CaseStatus = 'pass' | 'policy-adapted' | 'unsupported-by-design' | 'fail' | 'invalid-baseline'

interface Specification {
  id: string
  groups: string[]
  directExpected?: unknown
  proxyExpected?: unknown
  policyChange?: boolean
}

export interface CaseResult extends Specification {
  status: CaseStatus
  direct?: unknown
  proxy?: unknown
  error?: string
}

/** How long one case may run in one mode. */
const CASE_MS = 30_000

/** Runs `work`, failing after `ms`. */
async function deadline<T>(work: Promise<T>, ms: number, what: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined
  try {
    return await Promise.race([
      work,
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${what} took longer than ${ms} ms`)), ms)
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

/** Loads the lab again in `frame` and waits until its cases are ready. */
async function fresh(frame: Frame, navigate: () => Promise<unknown>): Promise<void> {
  // The old document's mark tells it from the new one, whose address may be the same.
  // A frame between documents has none to mark, and the new one has no mark either.
  await frame.evaluate(() => Reflect.set(window, '__labOld', true)).catch(() => {})
  await navigate()
  await frame.waitForFunction(
    () => Reflect.get(window, '__labOld') !== true && Reflect.get(window, 'labReady') === true,
    null,
    { timeout: CASE_MS },
  )
}

/** One case's answer in `frame`, or why it threw. */
async function runCase(frame: Frame, id: string): Promise<{ result?: unknown; error?: string }> {
  try {
    const result = await deadline(
      frame.evaluate((name): Promise<unknown> => Reflect.get(window, 'runLabCase')(name), id),
      CASE_MS,
      id,
    )
    return { result }
  } catch (error) {
    return { error: error instanceof Error ? error.message : String(error) }
  }
}

/**
 * Runs the lab's cases whose id contains one of `filter`'s, every one
 * without a filter, and writes the results to `report`. `page` shows the
 * conversation, its panel open.
 */
export async function previewLab(
  { page, context, report, filter = [], ports }: {
    page: Page
    context: BrowserContext
    report: string
    filter?: string[]
    ports: { app: number; aux: number; secondAux: number }
  },
  log: (line: string) => void,
): Promise<Record<CaseStatus, number>> {
  const sites = await startLab(ports)
  const browser = context.browser()
  if (!browser) {
    throw new Error('The lab needs the browser around the product’s page')
  }
  try {
    const lab = `${sites.app}/lab`
    // A tab of the user's browser of its own, on the lab.
    await page.getByRole('button', { name: 'New Tab' }).click()
    const address = page.getByRole('textbox', { name: 'Browser address' })
    await address.fill(lab)
    await address.press('Enter')
    /** The tab's frame, found again when the page around it loaded again. */
    let proxied: Frame | null = null
    const frame = async (): Promise<Frame> => {
      if (proxied && !proxied.isDetached()) {
        return proxied
      }
      const element = await page.locator('iframe[title="Page"]:visible').elementHandle()
      proxied = (await element?.contentFrame()) ?? null
      if (!proxied) {
        throw new Error('The tab of the user’s browser shows no frame')
      }
      return proxied
    }
    const shown = await frame()
    await shown.waitForFunction(() => Reflect.get(window, 'labReady') === true, null, { timeout: 60_000 })
    const catalog = (await shown.evaluate((): Specification[] => Reflect.get(window, 'labCatalog')))
      .filter((test) => filter.length === 0 || filter.some((part) => test.id.includes(part)))
    const cdp = await context.newCDPSession(page)
    // The baseline renders as the preview's frame does, whatever the product's page emulates.
    const deviceScaleFactor = await shown.evaluate(() => window.devicePixelRatio)
    const results: CaseResult[] = []
    for (const specification of catalog) {
      // Each baseline starts with nothing stored, as a case's first load in the preview does.
      const isolated = await browser.newContext({ deviceScaleFactor })
      let baseline: { result?: unknown; error?: string }
      try {
        const direct = await isolated.newPage()
        await direct.goto(lab)
        await direct.waitForFunction(() => Reflect.get(window, 'labReady') === true, null, { timeout: CASE_MS })
        baseline = await runCase(direct.mainFrame(), specification.id)
      } catch (error) {
        baseline = { error: error instanceof Error ? error.message : String(error) }
      } finally {
        await isolated.close()
      }
      let preview: { result?: unknown; error?: string }
      try {
        const current = await frame()
        // And each preview starts with nothing the earlier cases stored in the page's origin.
        // The forwarder's registration stays, as it does across a user's visits.
        await cdp.send('Storage.clearDataForOrigin', {
          origin: new URL(current.url()).origin,
          storageTypes: 'indexeddb,local_storage,cache_storage,file_systems,websql',
        })
        await fresh(current, async () => {
          await address.fill(lab)
          await address.press('Enter')
        })
        preview = await runCase(current, specification.id)
      } catch (error) {
        preview = { error: error instanceof Error ? error.message : String(error) }
      }
      const row: CaseResult = { ...specification, status: 'pass', direct: baseline.result ?? baseline.error, proxy: preview.result ?? preview.error }
      if (baseline.error === undefined && 'directExpected' in specification) {
        try {
          deepStrictEqual(baseline.result, specification.directExpected)
        } catch (error) {
          baseline.error = `the direct load answered otherwise than its fixture says: ${(error as Error).message}`
        }
      }
      if (baseline.error !== undefined) {
        row.status = 'invalid-baseline'
        row.error = baseline.error
      } else if (/ServiceWorker is unavailable in a Demi preview/.test(preview.error ?? '')) {
        // The site's own service worker would replace the forwarder: a preview declines it by design.
        row.status = 'unsupported-by-design'
      } else if (preview.error !== undefined) {
        row.status = 'fail'
        row.error = preview.error
      } else {
        try {
          deepStrictEqual(preview.result, 'proxyExpected' in specification ? specification.proxyExpected : baseline.result)
          const equivalent = JSON.stringify(preview.result) === JSON.stringify(baseline.result)
          row.status = specification.policyChange && !equivalent ? 'policy-adapted' : 'pass'
        } catch (error) {
          row.status = 'fail'
          row.error = (error as Error).message.slice(0, 2000)
        }
      }
      results.push(row)
      log(`${row.status.toUpperCase()} ${row.id}${row.error ? `: ${row.error.replaceAll('\n', ' ').slice(0, 160)}` : ''}`)
      writeFileSync(report, JSON.stringify(results, null, 2))
    }
    const counts = { pass: 0, 'policy-adapted': 0, 'unsupported-by-design': 0, fail: 0, 'invalid-baseline': 0 }
    for (const row of results) {
      counts[row.status]++
    }
    return counts
  } finally {
    await sites.close()
  }
}
