// Finding what a command acts on, and failing well when it is not there
// (browse.md § Finding things): a locator must match exactly one
// element; when it matches none or several, the failure says what it looked
// for, what the page has instead, and where a screenshot of the page is.
import { writeFileSync } from 'node:fs'
import { errors, type Locator, type Page } from 'playwright'
import { CommandFailure, type Context } from './command'
import { locate, parseTarget, type Target } from './locator'
import { shotPath } from './shots'

/**
 * How long an acting command lets a locator come to match, such as a menu
 * item while its menu opens: brief, so a wrong locator fails at once rather
 * than after Playwright's half minute.
 */
const SETTLE_MS = 2_000

export type Resolved =
  | { kind: 'point', x: number, y: number }
  | { kind: 'element', locator: Locator, text: string }

/** The point or the one element `text` names in the page. */
export async function resolve(context: Context, text: string, settleMs = SETTLE_MS): Promise<Resolved> {
  const target = parseLocatorText(text)
  if (target.kind === 'point') {
    return target
  }
  return { kind: 'element', locator: await one(context, target, settleMs), text }
}

/** The one element `text` names, or a failure; never a point. */
export async function element(context: Context, text: string, settleMs = SETTLE_MS): Promise<Locator> {
  const target = parseLocatorText(text)
  if (target.kind === 'point') {
    throw new CommandFailure(`${text} is a point; this needs an element's locator`)
  }
  return one(context, target, settleMs)
}

function parseLocatorText(text: string): Target {
  try {
    return parseTarget(text)
  } catch (error) {
    throw new CommandFailure(error instanceof Error ? error.message : String(error))
  }
}

async function one(context: Context, target: Extract<Target, { kind: 'locator' }>, settleMs: number): Promise<Locator> {
  const page = await context.browser.page()
  const locator = locate(page, target.segments)
  let count = await countOf(locator, target.text)
  if (count === 0) {
    try {
      await locator.first().waitFor({ state: 'attached', timeout: settleMs })
    } catch (error) {
      // Still nothing after the brief wait: the failure below says so.
      if (!(error instanceof errors.TimeoutError)) {
        throw error
      }
    }
    count = await countOf(locator, target.text)
  }
  if (count === 1) {
    return locator
  }
  if (count === 0) {
    throw await failure(context, [`Nothing matches ${target.text} at ${page.url()}.`, ...await nearby(page, target.text)])
  }
  throw await failure(context, [`${count} elements match ${target.text}; it must name one.`, ...await describeAll(locator)])
}

async function countOf(locator: Locator, text: string): Promise<number> {
  try {
    return await locator.count()
  } catch (error) {
    // Playwright refuses a selector it cannot parse, and says why.
    throw new CommandFailure(`${text} is not a locator: ${error instanceof Error ? error.message.split('\n')[0] : error}`)
  }
}

/**
 * Fails plainly when something covers the middle of `locator`, where a
 * click or the pointer would land, such as a dialog or its backdrop that
 * is still open: the failure names what covers it, rather than leaving
 * Playwright to retry until its timeout and print its call log. A target
 * that is not ready for another reason, such as one still disabled, is left
 * to the action's own wait.
 */
export async function uncovered(context: Context, locator: Locator, text: string): Promise<void> {
  try {
    // A trial click waits until the element is shown, steady, enabled and the one a click would reach.
    await locator.click({ trial: true, timeout: SETTLE_MS })
    return
  } catch (error) {
    if (!(error instanceof errors.TimeoutError)) {
      throw error
    }
  }
  const cover = await locator.evaluate(coverOf)
  if (cover !== null) {
    throw await failure(context, [`${text} is covered by ${cover}, which would take the click. Close or finish it first, or act on it instead.`])
  }
}

/** What covers the middle of `node`, in words, or null when nothing does. Runs in the page. */
function coverOf(node: Element): string | null {
  const box = node.getBoundingClientRect()
  if (box.width === 0 || box.height === 0) {
    return null
  }
  const hit = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2)
  if (hit === null || hit === node || node.contains(hit)) {
    return null
  }
  const named = (element: Element): string => {
    const labelledBy = element.getAttribute('aria-labelledby')
    const label = element.getAttribute('aria-label')
      ?? (labelledBy === null ? null : document.getElementById(labelledBy)?.textContent)
      ?? ''
    return label.trim().replace(/\s+/g, ' ')
  }
  const describe = (element: Element): string => {
    const role = element.getAttribute('role')
    const words = named(element) || (element.textContent ?? '').trim().replace(/\s+/g, ' ')
    const shown = words.length > 40 ? `${words.slice(0, 40)}…` : words
    return `<${element.tagName.toLowerCase()}${role === null ? '' : ` role=${role}`}>${shown === '' ? '' : ` "${shown}"`}`
  }
  const DIALOG = '[role=dialog], [role=alertdialog], dialog, [aria-modal=true]'
  const around = hit.closest(DIALOG)
  if (around !== null) {
    return around === hit ? `the dialog "${named(around)}"` : `${describe(hit)} in the dialog "${named(around)}"`
  }
  // A modal's backdrop holds the dialog it covers the page for.
  const within = hit.querySelector(DIALOG)
  return within === null ? describe(hit) : `the backdrop of the dialog "${named(within)}"`
}

/**
 * A failure with `lines` and a screenshot of the page as it is, which the
 * caller can look at without running anything else.
 */
export async function failure(context: Context, lines: string[]): Promise<CommandFailure> {
  if (await context.browser.existingPage() !== null) {
    const path = shotPath(context.slot, `failure-${Date.now()}`)
    try {
      writeFileSync(path, await context.browser.capture())
      lines.push(`Screenshot: ${path}`)
    } catch (error) {
      lines.push(`No screenshot: ${error instanceof Error ? error.message.split('\n')[0] : error}`)
    }
  }
  return new CommandFailure(lines.join('\n'))
}

/** What the page offers that resembles `text`: its elements of the same role, or with the same words. */
async function nearby(page: Page, text: string): Promise<string[]> {
  let snapshot: string
  try {
    snapshot = await page.locator('body').ariaSnapshot({ timeout: 5_000 })
  } catch {
    // A page without a body, such as one still loading, offers nothing to list.
    return ['The page has no content yet.']
  }
  // A sign-in form's password field shows its value in the snapshot.
  const lines = snapshot.split('\n').map((line) => line.replace(/^(\s*- textbox "Password[^"]*"):.*$/, '$1: ••••'))
  const role = /^role=([a-z]+)/.exec(text)?.[1]
  const words = text.replace(/^[a-z]+~?=/, '').replace(/[[\]"'=]/g, ' ').toLowerCase().split(/\s+/).filter((word) => word.length > 2)
  const similar = lines.filter((line) => {
    const lower = line.toLowerCase()
    if (role) {
      return lower.trimStart().startsWith(`- ${role}`)
    }
    return words.some((word) => lower.includes(word))
  })
  const shown = similar.length > 0 ? similar : lines
  const head = shown.slice(0, 40).map((line) => `  ${line.trim()}`)
  const rest = shown.length > 40 ? [`  … and ${shown.length - 40} more`] : []
  const title = similar.length > 0 ? 'The page has instead:' : 'The page has:'
  return [title, ...head, ...rest]
}

async function describeAll(locator: Locator): Promise<string[]> {
  const count = await locator.count()
  const lines: string[] = []
  for (let index = 0; index < Math.min(count, 10); index += 1) {
    const nth = locator.nth(index)
    const description = await nth.evaluate((node) => {
      const label = node.getAttribute('aria-label') ?? (node.textContent ?? '').trim().slice(0, 60)
      return `<${node.tagName.toLowerCase()}> ${JSON.stringify(label)}`
    })
    const box = await nth.boundingBox()
    const where = box ? ` at ${Math.round(box.x)},${Math.round(box.y)} ${Math.round(box.width)}×${Math.round(box.height)}` : ' (not shown)'
    lines.push(`  ${index + 1}. ${description}${where}`)
  }
  if (count > 10) {
    lines.push(`  … and ${count - 10} more`)
  }
  lines.push('Narrow it, for example with `>>` inside a dialog or a region, or with exact text in quotes.')
  return lines
}
