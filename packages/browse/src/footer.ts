// What a call prints after its script's output (browse.md § A call): the
// screenshots it wrote, then where the page stands, so the agent rarely
// needs another call only to see it: the page's address and title, a dialog
// or menu that covers it, the focused element, and the console errors and
// failed requests since the call began. Playwright names the layer as its
// accessibility snapshot does, by role and name. The focused element is
// named as the browser's own accessibility tree has it, which also knows an
// editable element with no role of its own, such as the composer's.
import type { CDPSession, Locator, Page } from 'playwright'
import type { Shot } from './tool'

export interface PageState {
  url: string
  title: string
  /** The topmost dialog or menu, such as `dialog "Add Device"`, or null. */
  layer: string | null
  /** The focused element, such as `button "Stop"` or `generic "Message", editable`, or null when the page itself has the focus. */
  focus: string | null
}

export interface CallReport {
  shots: Shot[]
  /** The page's state, or null when the call left no browser. */
  page: PageState | null
  /** The console errors and failed requests since the call began. */
  problems: { console: string[], network: string[] }
}

/** How many lines of each kind of problem the footer shows; the rest are counted. */
const PROBLEMS = 10
/** How much of a console line the footer shows. */
const LINE = 300
/** How long reading an element's accessible name may take. */
const READ_MS = 2_000

/** The footer's lines. */
export function composeFooter(report: CallReport): string[] {
  const lines: string[] = []
  const pathWidth = Math.max(0, ...report.shots.map((shot) => shot.path.length))
  for (const shot of report.shots) {
    lines.push(`${shot.kind.padEnd(9)} ${shot.path.padEnd(pathWidth)}  ${shot.detail}`)
  }
  if (report.page) {
    const { url, title, layer, focus } = report.page
    lines.push(`Page      ${url}${title === '' ? '' : ` · ${JSON.stringify(title)}`}`)
    if (layer !== null) {
      lines.push(`Layer     ${layer}`)
    }
    lines.push(`Focus     ${focus ?? 'nothing (the page)'}`)
  }
  lines.push(...listed('Console', report.problems.console))
  lines.push(...listed('Requests', report.problems.network))
  return lines
}

/** `entries` under `label`, the label on the first line only, the first few of them. */
function listed(label: string, entries: string[]): string[] {
  const shown = entries.slice(0, PROBLEMS).map((entry, index) => {
    const text = entry.length > LINE ? `${entry.slice(0, LINE)}…` : entry
    return `${(index === 0 ? label : '').padEnd(9)} ${text}`
  })
  if (entries.length > PROBLEMS) {
    shown.push(`${''.padEnd(9)} … and ${entries.length - PROBLEMS} more`)
  }
  return shown
}

/** Where `page` stands now; `cdp` is a session with it. */
export async function readPageState(page: Page, cdp: CDPSession): Promise<PageState> {
  const layers = page.getByRole('dialog').or(page.getByRole('alertdialog')).or(page.getByRole('menu'))
  return {
    url: page.url(),
    title: await page.title(),
    layer: await describeLayer(layers),
    focus: await describeFocus(cdp),
  }
}

/** The last element `locator` matches, by role and name, or null when it matches none. */
async function describeLayer(locator: Locator): Promise<string | null> {
  if (await locator.count() === 0) {
    return null
  }
  const snapshot = await locator.last().ariaSnapshot({ timeout: READ_MS })
  // The element's own line, such as `- dialog "Add Device":` above its contents.
  return shorter(snapshot.split('\n')[0].replace(/^\s*- /, '').replace(/:$/, ''))
}

/** The focused element by its role and name, or null when the page's body has the focus. */
async function describeFocus(cdp: CDPSession): Promise<string | null> {
  // The body has the focus when no element does.
  const expression = 'document.activeElement === document.body ? null : document.activeElement'
  const { result } = await cdp.send('Runtime.evaluate', { expression })
  if (result.objectId === undefined) {
    return null
  }
  try {
    const { nodes } = await cdp.send('Accessibility.getPartialAXTree', { objectId: result.objectId, fetchRelatives: false })
    const node = nodes[0]
    if (node === undefined || node.ignored) {
      return null
    }
    const role = String(node.role?.value ?? 'element')
    const name = node.name?.value ? ` ${JSON.stringify(String(node.name.value))}` : ''
    const editable = node.properties?.some((property) => property.name === 'editable') ? ', editable' : ''
    return shorter(`${role}${name}${editable}`)
  } finally {
    await cdp.send('Runtime.releaseObject', { objectId: result.objectId })
  }
}

function shorter(text: string): string {
  return text.length > 120 ? `${text.slice(0, 120)}…` : text
}
