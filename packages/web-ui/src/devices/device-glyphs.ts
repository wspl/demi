import { h, type Component, type FunctionalComponent } from 'vue'
import DeviceIcon from './DeviceIcon.vue'
import { deviceIcon } from '../targets/icons'
import type { TargetKind } from '../targets/types'
import type { DeviceMark } from './mark'
import type { DeviceState } from './state'

/** Each mark and state drawn once, so a row that draws it again keeps its icon. */
const glyphs = new Map<string, FunctionalComponent>()

/**
 * A device's mark with its state as an icon a row takes by component, such as a menu row's:
 * `DeviceIcon`, so a device looks the same wherever it is drawn.
 */
export function deviceGlyph(state: DeviceState | null, mark: DeviceMark | null): FunctionalComponent {
  const key = `${state}:${mark?.logo}:${mark?.number}`
  let glyph = glyphs.get(key)
  if (!glyph) {
    glyph = () => h(DeviceIcon, { state, mark })
    glyphs.set(key, glyph)
  }
  return glyph
}

/** A device as an icon a row takes by component: the Cloud's glyph, or a paired device's mark with its state. */
export function targetGlyph(target: { kind: TargetKind; state: DeviceState; mark: DeviceMark | null }): Component {
  return target.kind === 'cloud' ? deviceIcon(target) : deviceGlyph(target.state, target.mark)
}
