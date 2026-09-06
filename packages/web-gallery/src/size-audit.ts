/**
 * Finds settings cards whose controls mix height families. Inputs, buttons, icon
 * buttons and segmented controls come in one family per surface (24px in settings
 * cards); a 24px button beside a 28px input is a mistake, not a choice. See
 * docs/demi-next/web-prototype.md.
 */
export interface SizeFinding {
  card: Element
  heights: number[]
  elements: Element[]
}

const CONTROL = '[role="button"], [role="radiogroup"], input, textarea, select'
const FAMILIES = [20, 24, 28]

export function auditControlSizes(root: ParentNode = document.body): SizeFinding[] {
  const findings: SizeFinding[] = []
  for (const card of root.querySelectorAll('.settings-card')) {
    const seen = new Map<number, Element[]>()
    for (const el of card.querySelectorAll(CONTROL)) {
      // Switches and checkboxes have their own scale; text inputs measure by their frame.
      if (el.getAttribute('role') === 'switch' || el.getAttribute('role') === 'checkbox' || (el as HTMLInputElement).type === 'checkbox') continue
      // A control inside an input's frame (the eye on a secret) belongs to the input, not the row.
      if (el.tagName !== 'INPUT' && el.closest(':has(> input)')) continue
      const box = el.tagName === 'INPUT' ? (el.parentElement ?? el) : el
      const h = Math.round(box.getBoundingClientRect().height)
      if (!FAMILIES.includes(h)) continue
      seen.set(h, [...(seen.get(h) ?? []), el])
    }
    if (seen.size > 1) {
      const heights = [...seen.keys()].sort((a, b) => a - b)
      findings.push({ card, heights, elements: heights.flatMap((h) => seen.get(h) ?? []) })
    }
  }
  return findings
}

function describe(el: Element): string {
  const label = el.getAttribute('aria-label') ?? el.textContent?.trim().slice(0, 24) ?? ''
  return `${el.tagName.toLowerCase()}${label ? ` "${label}"` : ''}`
}

export function reportControlSizes(root?: ParentNode): number {
  const findings = auditControlSizes(root)
  if (findings.length) {
    console.warn(`[size-audit] ${findings.length} card(s) mix control heights`)
    for (const f of findings) {
      const title = f.card.parentElement?.querySelector('h3')?.textContent?.trim() ?? '(untitled card)'
      console.warn(`  ${title}: ${f.heights.join(' / ')}px`, f.elements.map(describe).join(', '))
    }
  }
  return findings.length
}

declare global {
  interface Window {
    demiAuditControlSizes: (root?: ParentNode) => SizeFinding[]
  }
}

export function installSizeAudit(afterNavigation: (run: () => void) => void): void {
  window.demiAuditControlSizes = auditControlSizes
  if (import.meta.env.DEV) afterNavigation(() => window.setTimeout(() => reportControlSizes(), 1500))
}
