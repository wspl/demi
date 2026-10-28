// How the demos bring in what the app shows at once: the work panel, a side
// question's card, a panel's or a search's content once loaded. Demi itself
// puts them in place in one frame, as a tool should; a demo is watched, not
// used, and a part that pops in reads as a glitch there. Each animation is
// the page's own, so a recording keeps it and its play runs it.

/** The curve a panel or a card settles in on, as a sheet does in macOS. */
const SETTLE = 'cubic-bezier(0.32, 0.72, 0, 1)'

interface Entrance {
  selector: string
  /** Whether it is found inside an added part as well, not only as the part itself. */
  within?: boolean
  /** Whether a part that matches is brought in: some are too small to need it. */
  when?(element: HTMLElement): boolean
  run(element: HTMLElement): void
}

function fade(element: HTMLElement, ms: number): void {
  element.animate([{ opacity: 0 }, { opacity: 1 }], { duration: ms, easing: 'ease-out' })
}

const ENTRANCES: readonly Entrance[] = [
  {
    // The work panel slides in from the window's right edge, and the
    // conversation gives it room as it comes; its divider, which keeps
    // windows too, comes with it.
    selector: '[data-region][data-keeps-windows]',
    within: true,
    run: (panel) => {
      panel.animate([{ marginRight: `${-panel.offsetWidth}px` }, { marginRight: '0px' }], { duration: 500, easing: SETTLE })
    },
  },
  {
    // A side question's card opens under the words it asks about.
    selector: '[data-ask-card]',
    within: true,
    run: (card) => {
      card.animate(
        [{ height: '0px', opacity: 0, overflow: 'hidden' }, { height: `${card.offsetHeight}px`, opacity: 1, overflow: 'hidden' }],
        { duration: 350, easing: SETTLE },
      )
    },
  },
  {
    // A panel's content, once read: the diff after "Reading Changes".
    selector: '[data-work-panel] *',
    when: (element) => element.offsetHeight > 160 && element.offsetWidth > 160,
    run: (content) => fade(content, 250),
  },
  {
    // A search's results, after "Searching…".
    selector: '[role="dialog"] *',
    when: (element) => element.offsetHeight > 40 && element.offsetWidth > 200,
    run: (results) => fade(results, 200),
  },
]

/** Brings in each part that `root` gains, as `ENTRANCES` says; answers how to stop. */
export function animateEntrances(root: Element): () => void {
  const observer = new MutationObserver((records) => {
    const added = new Set<HTMLElement>()
    for (const record of records) {
      for (const node of record.addedNodes) {
        if (node instanceof HTMLElement)
          added.add(node)
      }
    }
    for (const node of added) {
      // An added part's own parts come in with it.
      if ([...added].some((other) => other !== node && other.contains(node)))
        continue
      for (const entrance of ENTRANCES) {
        const parts = node.matches(entrance.selector)
          ? [node]
          : entrance.within ? [...node.querySelectorAll<HTMLElement>(entrance.selector)] : []
        const shown = parts.filter((part) => entrance.when?.(part) ?? true)
        for (const part of shown)
          entrance.run(part)
        if (shown.length > 0)
          break
      }
    }
  })
  observer.observe(root, { childList: true, subtree: true })
  return () => observer.disconnect()
}
