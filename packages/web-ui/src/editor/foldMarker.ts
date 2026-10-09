import { h, render } from 'vue'
import FoldChevron from '../ui/FoldChevron.vue'

// CodeMirror asks the fold gutter for a new element whenever a foldable line
// comes into view and never says when it drops one, so a marker cannot hold a
// live component. The app's fold chevron renders once per state, and every
// marker is a copy.
const templates = new Map<boolean, HTMLElement>()

function template(open: boolean): HTMLElement {
  const cached = templates.get(open)
  if (cached)
    return cached
  const rendered = document.createElement('span')
  rendered.className = 'flex size-[18px] items-center justify-center rounded-[3px] text-fg-subtle hover:bg-active'
  render(h(FoldChevron, { open }), rendered)
  const copy = document.importNode(rendered, true)
  render(null, rendered)
  templates.set(open, copy)
  return copy
}

/** The fold gutter's marker: pointing down on a line that folds, right on a folded one. */
export function foldMarker(open: boolean): HTMLElement {
  return document.importNode(template(open), true)
}
