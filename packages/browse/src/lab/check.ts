/**
 * The web preview's behavior laboratory as a check (`preview.md` § Tests),
 * which a `bun browse` call runs:
 *
 *   bun browse --limit 120m <<'JS'
 *   const { previewLabCheck } = await import(`${process.cwd()}/packages/browse/src/lab/check.ts`)
 *   console.log(JSON.stringify(await previewLabCheck({ demi, page, context, filter: [] })))
 *   JS
 *
 * It starts the slot's backend, web app and runner, opens a conversation on
 * the runner with a first message, which needs a model, and runs the lab's
 * cases whose id contains a part of `filter`, every one for an empty one,
 * directly and through a tab of the user's browser. The results go to
 * `.cache/browse/preview-lab.json`, and each case's line to the call's
 * output.
 */
import type { BrowserContext, Page } from 'playwright'
import { previewLab } from './run'

/** The helpers of `bun browse` the check uses. */
interface Demi {
  up(): Promise<unknown>
  runner(): Promise<string | null>
  message(text: string): Promise<unknown>
}

export async function previewLabCheck({ demi, page, context, filter = [] }: { demi: Demi; page: Page; context: BrowserContext; filter?: string[] }) {
  await demi.up()
  const device = await demi.runner()
  if (device === null) {
    throw new Error('The slot’s runner does not run')
  }
  await page.goto('/')
  await page.getByRole('button', { name: 'Manage conversation hosts' }).click()
  await page.getByRole('menuitem', { name: /Primary Host/ }).hover()
  await page.getByRole('menuitem', { name: device }).click()
  await demi.message('Reply with the single word: ready')
  const folder = page.getByRole('button', { name: 'Use This Folder' })
  if (await folder.isVisible()) {
    await folder.click()
  }
  await page.getByRole('button', { name: 'Open panel' }).click()
  const root = process.cwd()
  // The slot's own ports, 189n6 to 189n8.
  const slot = Number(/demi-slots\/(\d+)/.exec(root)?.[1] ?? 0)
  const ports = { app: 18906 + slot * 10, aux: 18907 + slot * 10, secondAux: 18908 + slot * 10 }
  return previewLab({ page, context, report: `${root}/.cache/browse/preview-lab.json`, filter, ports }, (line) => console.log(line))
}
