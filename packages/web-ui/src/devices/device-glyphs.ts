import { h, type Component, type FunctionalComponent } from 'vue'
import DeviceIcon from './DeviceIcon.vue'
import { hostIcon } from '../hosts/icons'
import type { HostKind } from '../hosts/types'
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

/** A host as an icon a row takes by component: the Cloud's glyph, or a device's mark with its state. */
export function hostGlyph(host: { kind: HostKind; state: DeviceState; mark: DeviceMark | null }): Component {
  return host.kind === 'cloud' ? hostIcon(host) : deviceGlyph(host.state, host.mark)
}
