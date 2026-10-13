// `demi.focusRings({ root, name, shots })`: finds focus indicators the page
// does not show whole. It walks the page with Tab, and focuses what Tab
// skips (tabindex -1) through the page, and for each element the keyboard
// focuses it takes the indicator's box (an outline with its offset, or a
// crisp ring of box shadow) and reports the part an ancestor cuts off
// (`overflow` other than visible, `contain: paint`, `clip-path`, the window's
// edge) and the part an element painted above covers, with how many
// pixels on which side. Each offender is photographed with the focus on it.
// `root` limits the elements checked to those inside it; the page is checked
// as it stands, so a dialog or a menu is opened first.
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
  /** `Tab` when the Tab key reached it, else `focus()`. */
  reached: 'Tab' | 'focus()'
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

export async function focusRings(tool: Tool, options: FocusRingsOptions = {}): Promise<FocusRingsReport> {
  const page = await tool.browser.page()
  const root = options.root ?? null
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

  const take = async (reached: RingOffender['reached'], reading: RingReading) => {
    if (reading.findings.length === 0) {
      return
    }
    const offender: RingOffender = { element: reading.element, indicator: reading.indicator, reached, findings: reading.findings }
    if ((options.shots ?? true) && shots < MOST_SHOTS) {
      shots++
      const box = reading.box
      const left = Math.max(0, box.left - MARGIN)
      const top = Math.max(0, box.top - MARGIN)
      offender.shot = await shot(tool, `${options.name ?? 'ring'}-${String(report.offenders.length + 1).padStart(2, '0')}`, {
        region: { x: left, y: top, width: Math.max(1, box.right + MARGIN - left), height: Math.max(1, box.bottom + MARGIN - top) },
        zoom: 3,
        now: true,
      })
    }
    report.offenders.push(offender)
  }
  const reading = async (reached: RingOffender['reached']) => {
    const result = await call('read')
    if (result === 'skipped') {
      return
    }
    report.checked++
    if (result === 'none') {
      report.withoutIndicator.push(await page.evaluate(() => {
        const active = document.activeElement
        return active ? `${active.tagName.toLowerCase()} ${(active.getAttribute('aria-label') ?? active.textContent ?? '').trim().slice(0, 40)}` : ''
      }))
      return
    }
    await take(reached, result)
  }

  const count = await call('candidates', root)
  if (count === 0) {
    throw new Failure('demi.focusRings found nothing the keyboard can focus on the page')
  }
  // The keyboard is the last input, so the browser matches :focus-visible on what the page focuses next.
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
  await page.keyboard.press('Shift')
  for (let presses = 0; presses < MOST_TABS; presses++) {
    await page.keyboard.press('Tab')
    if (await call('seen')) {
      break
    }
    await reading('Tab')
  }
  for (const index of await call('unreached')) {
    if (await call('focusCandidate', index)) {
      await reading('focus()')
    }
  }
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
  await page.evaluate(() => Reflect.deleteProperty(window, '__demiRings'))

  tool.print(`focus rings: ${report.checked} checked, ${report.offenders.length} cut or covered, ${report.withoutIndicator.length} without an indicator of their own`)
  for (const offender of report.offenders) {
    for (const finding of offender.findings) {
      const cuts = (Object.entries(finding.cuts) as [string, number][]).filter(([, amount]) => amount > 0).map(([side, amount]) => `${side} ${amount} px`).join(', ')
      tool.print(`  ${finding.kind}  ${offender.element}  [${offender.indicator}; ${offender.reached}]  ${cuts}  by ${finding.by} (${finding.reason}${finding.scrolls ? '; scrolls' : ''})${offender.shot ? `  ${offender.shot}` : ''}`)
    }
  }
  return report
}
