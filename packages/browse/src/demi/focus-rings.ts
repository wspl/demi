// `demi.focusRings({ root, name, shots })`: finds focus indicators the page
// does not show whole. It walks the page with Tab, and focuses what Tab
// skips (tabindex -1) through the page, and for each element the keyboard
// focuses it takes the indicator's box (an outline with its offset, or a
// crisp ring of box shadow) and reports the part an ancestor cuts off
// (`overflow` other than visible, `contain: paint`, `clip-path`, the window's
// edge) and the part an element painted above covers, with how many
// pixels on which side; an element that is itself under another one is
// reported as hidden, since its focus is out of sight, not only its
// indicator. Each offender is photographed with the focus on it. `root`
// limits the elements checked to those inside it; the page is checked as it
// stands, so a dialog or a menu is opened first, and one that floats over the
// page is the only part checked, as it is the only part a person reaches.
//
// `demi.focusRings.of(locator)` reads the indicator an element shows now,
// for one a state draws that a walk with Tab does not reach, such as the
// outline of a selected capsule in a message.
import type { Locator } from 'playwright'
import { shot } from './shot'
import { ringProbe, type RingFinding, type RingProbe, type RingReading } from './focus-rings-probe'
import { Failure, type Tool } from '../tool'

export interface FocusRingsOptions {
  /** A CSS selector: only elements inside it are reported. */
  root?: string
  /** A prefix of the screenshots' names, or a folder with it, such as `/…/shots/settings`. */
  name?: string
  /** Photograph the offenders; on by default, at most 40 per call. */
  shots?: boolean
}

export interface RingOffender {
  element: string
  indicator: string
  /** `Tab` when the Tab key reached it, `focus()` when the page focused it, `shown` when the script asked for it. */
  reached: 'Tab' | 'focus()' | 'shown'
  findings: RingFinding[]
  shot?: string
}

export interface FocusRingsReport {
  /** The elements whose indicator was read. */
  checked: number
  /** Focusable elements that show no indicator of their own when the keyboard focuses them. */
  withoutIndicator: string[]
  offenders: RingOffender[]
}

/** The most pictures a call takes; the report still lists every offender. */
const MOST_SHOTS = 40
/** The most Tab presses of one walk, for a page whose focus never comes round. */
const MOST_TABS = 600
/** What a picture shows around the indicator, in CSS pixels. */
const MARGIN = 24

/** The probe in the page of this call, and what a call does with what it reads. */
async function start(tool: Tool, options: FocusRingsOptions) {
  const page = await tool.browser.page()
  await page.evaluate((source) => {
    Object.assign(window, { __demiRings: (0, eval)(`(${source})`)() })
  }, ringProbe.toString())
  // The probe lives in the page; a method is called there by name.
  const call = <K extends keyof RingProbe>(method: K, ...args: Parameters<RingProbe[K]>): Promise<Awaited<ReturnType<RingProbe[K]>>> =>
    page.evaluate(([name, values]) => {
      const rings: Record<string, (...given: unknown[]) => unknown> = Reflect.get(window, '__demiRings')
      return rings[name](...values)
    }, [method, args] as const) as Promise<Awaited<ReturnType<RingProbe[K]>>>
  const report: FocusRingsReport = { checked: 0, withoutIndicator: [], offenders: [] }
  let shots = 0

  return {
    page,
    call,
    report,
    /** Files what a reading found: its offender, with the picture of the indicator. */
    async take(reached: RingOffender['reached'], reading: RingReading): Promise<void> {
      report.checked++
      if (reading.findings.length === 0) {
        return
      }
      const offender: RingOffender = { element: reading.element, indicator: reading.indicator, reached, findings: reading.findings }
      if ((options.shots ?? true) && shots < MOST_SHOTS) {
        shots++
        // The picture shows the indicator and what is around it, within the window.
        const window = await page.evaluate(() => ({ width: document.documentElement.clientWidth, height: document.documentElement.clientHeight }))
        const left = Math.max(0, reading.box.left - MARGIN)
        const top = Math.max(0, reading.box.top - MARGIN)
        const width = Math.max(1, Math.min(window.width, reading.box.right + MARGIN) - left)
        const height = Math.max(1, Math.min(window.height, reading.box.bottom + MARGIN) - top)
        offender.shot = await shot(tool, `${options.name ?? 'ring'}-${String(report.offenders.length + 1).padStart(2, '0')}`, {
          region: { x: left, y: top, width, height },
          // Small indicators are magnified, so that a pixel or two of cut shows.
          zoom: Math.max(width, height) <= 300 ? 3 : Math.max(width, height) <= 700 ? 2 : 1,
          now: true,
        })
      }
      report.offenders.push(offender)
    },
    /** Prints the report, and takes the probe out of the page. */
    async finish(): Promise<FocusRingsReport> {
      await page.evaluate(() => Reflect.deleteProperty(window, '__demiRings'))
      tool.print(`focus rings: ${report.checked} checked, ${report.offenders.length} cut or covered, ${report.withoutIndicator.length} without an indicator of their own`)
      for (const offender of report.offenders) {
        for (const finding of offender.findings) {
          const cuts = (Object.entries(finding.cuts) as [string, number][]).filter(([, amount]) => amount > 0).map(([side, amount]) => `${side} ${amount} px`).join(', ')
          tool.print(`  ${finding.kind}  ${offender.element}  [${offender.indicator}; ${offender.reached}]  ${cuts}  by ${finding.by} (${finding.reason}${finding.scrolls ? '; scrolls' : ''})${offender.shot ? `  ${offender.shot}` : ''}`)
        }
      }
      return report
    },
  }
}

export async function focusRings(tool: Tool, options: FocusRingsOptions = {}): Promise<FocusRingsReport> {
  const { page, call, report, take, finish } = await start(tool, options)
  const { count, layer } = await call('candidates', options.root ?? null)
  if (count === 0 && layer === null) {
    throw new Failure('demi.focusRings found nothing the keyboard can focus on the page')
  }
  const read = async (reached: RingOffender['reached']) => {
    const reading = await call('read')
    if (reading === 'none') {
      report.withoutIndicator.push(await page.evaluate(() => {
        const active = document.activeElement
        return active ? `${active.tagName.toLowerCase()} ${(active.getAttribute('aria-label') ?? active.textContent ?? '').trim().slice(0, 40)}` : ''
      }))
    } else if (reading !== 'skipped') {
      await take(reached, reading)
    }
  }
  // The keyboard is the last input, so the browser matches :focus-visible on what the page focuses next.
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
  await page.keyboard.press('Shift')
  // A menu moves its focus with the arrow keys and closes when Tab leaves it: what it holds is focused through the page.
  for (let presses = 0; presses < (layer === 'menu' ? 0 : MOST_TABS); presses++) {
    await page.keyboard.press('Tab')
    if (await call('seen')) {
      break
    }
    await read('Tab')
  }
  for (const index of await call('unreached')) {
    if (await call('focusCandidate', index)) {
      await read('focus()')
    }
  }
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
  return finish()
}

/** The indicator each element `locator` matches shows now. */
focusRings.of = async function of(tool: Tool, locator: Locator, options: FocusRingsOptions = {}): Promise<FocusRingsReport> {
  const { report, take, finish } = await start(tool, options)
  const readings = await locator.evaluateAll((elements) => {
    const rings: RingProbe = Reflect.get(window, '__demiRings')
    return Promise.all(elements.map((element) => rings.readShown(element)))
  })
  for (const reading of readings) {
    if (reading === 'none') {
      report.withoutIndicator.push(String(locator))
    } else {
      await take('shown', reading)
    }
  }
  return finish()
}
