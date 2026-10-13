import type { Directive } from 'vue'

/**
 * Aligns the drawn end of an icon-only control with its container's content
 * edge, where the figures and text beside it end. A control's box is larger
 * than its glyph, and a glyph's own drawing leaves room at its right (a
 * chevron more than a refresh arrow), so a box that ends at the content edge
 * leaves the glyph short of it. The element the directive is on ends where
 * its last icon's ink ends: it measures how far that ink stops short of the
 * element's end edge, once at mount while the icon rests, and pulls the
 * element's end back by that. The measure reads the icon's own geometry and
 * stroke, so no caller carries a number.
 */
export const vInkEdgeEnd: Directive<HTMLElement> = {
  mounted(el) {
    const icon = [...el.querySelectorAll('svg')].at(-1) ?? (el instanceof SVGSVGElement ? el : null)
    if (!icon || icon.viewBox.baseVal.width === 0)
      return
    const box = el.getBoundingClientRect()
    const drawn = icon.getBoundingClientRect()
    // A box scaled by an opening animation measures in scaled pixels.
    const scale = drawn.width / icon.clientWidth
    if (!Number.isFinite(scale) || scale === 0)
      return
    const { x, width } = icon.getBBox()
    const stroke = Number.parseFloat(getComputedStyle(icon).strokeWidth) || 0
    const inkRight = drawn.left + ((x + width + stroke / 2) / icon.viewBox.baseVal.width) * drawn.width
    el.style.marginInlineEnd = `${(inkRight - box.right) / scale}px`
  },
}
