import { h, type FunctionalComponent } from 'vue'

/**
 * A phone browser's tab button as an icon for `IconButton`: a rounded square in the
 * icon's stroke holding how many tabs there are, as Chrome's and Safari's tab
 * buttons draw theirs. With none it is the square alone, never a "0".
 *
 * It is drawn at the size the button gives a Lucide icon, its square as large as a
 * Lucide square in that box and its stroke as thick.
 */
export function tabCountIcon(count: number): FunctionalComponent<{ size?: number }> {
  return (props) => {
    const box = props.size ?? 20
    // A Lucide square spans 18 of its 24-unit box, its stroke 1.75 units.
    const side = Math.round(box * 18 / 24)
    const stroke = box * 1.75 / 24
    const shown = count > 99 ? '99+' : count > 0 ? String(count) : ''
    return h('span', {
      'aria-hidden': 'true',
      class: 'flex items-center justify-center',
      style: { width: `${box}px`, height: `${box}px` },
    }, h('span', {
      class: 'flex items-center justify-center font-semibold tabular-nums leading-none',
      style: {
        width: `${side}px`,
        height: `${side}px`,
        border: `${stroke}px solid currentColor`,
        borderRadius: `${Math.round(side * 0.27)}px`,
        fontSize: `${Math.round(side * (shown.length > 2 ? 0.42 : 0.62))}px`,
      },
    }, shown))
  }
}
