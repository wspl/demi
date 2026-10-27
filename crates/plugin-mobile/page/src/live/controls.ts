/**
 * The device toolbar's controls (`crates/plugin-mobile/page/src/index.ts`), in
 * their order: each the page method it calls and that method's parameters.
 * They run the operation of the `demi mobile` command that does the same, so
 * Rotate Left is `demi mobile rotate <m> left` and Lock is `demi mobile key
 * <m> lock`. The toolbar is drawn from this table and the controller runs its
 * calls.
 */
import type { Component } from 'vue'
import { ArrowLeft, BookOpen, Camera, Circle, Columns2, House, Lock, RectangleVertical, RotateCcw, RotateCw, Square, SunMoon, Volume1, Volume2 } from '@lucide/vue'
import type { FoldDevice, LiveFold, Posture, MobilePlatform, OnDevice, PressButton, RecordDevice, RotateDevice } from '../generated/plugin'

/** A page method's call: its name and parameters. */
export type DeviceCall =
  | { method: 'rotate'; params: RotateDevice }
  | { method: 'fold'; params: FoldDevice }
  | { method: 'toggleAppearance'; params: OnDevice }
  | { method: 'press'; params: PressButton }
  | { method: 'screenshot'; params: OnDevice }
  | { method: 'record'; params: RecordDevice }

/** The toolbar's names, keys of the page's text. */
export type ControlLabel =
  | 'toolbar.rotateLeft'
  | 'toolbar.rotateRight'
  | 'fold.closed'
  | 'fold.halfOpen'
  | 'fold.open'
  | 'toolbar.appearance'
  | 'toolbar.home'
  | 'toolbar.back'
  | 'toolbar.lock'
  | 'toolbar.volumeUp'
  | 'toolbar.volumeDown'
  | 'toolbar.screenshot'
  | 'toolbar.record'
  | 'toolbar.stopRecording'

/** A device's state the toolbar shows. */
export interface ControlState {
  recording: boolean
  /** How the device folds and is folded; none for one that does not fold. */
  fold: LiveFold | null
}

export interface DeviceControl {
  /** Its name and icon, as the device's state makes them. */
  label(state: ControlState): ControlLabel
  icon(state: ControlState): Component
  /** The call it makes for `device`. */
  call(device: string, state: ControlState): DeviceCall
  /** Whether a divider comes before it, starting a group. */
  group?: boolean
  /** Whether it shows as pressed, as Record does while recording. */
  pressed?(state: ControlState): boolean
  /** Whether its pressed state is a warning, the red dot of a recording that runs. */
  alerts?: boolean
  /** The one platform whose devices have it, as only Android has Back. */
  platform?: MobilePlatform
  /** The folds whose devices alone have it, as only a foldable has Closed and Open, and only a hinged one Half Open. */
  folds?: readonly LiveFold['kind'][]
}

function fixed(label: ControlLabel, icon: Component, call: (device: string) => DeviceCall, group = false): DeviceControl {
  return { label: () => label, icon: () => icon, call, group }
}

/** The hinge's angle of each posture of a hinged foldable. */
export const POSTURE_DEGREES: Readonly<Record<Posture, number>> = { 'closed': 0, 'half-open': 130, 'open': 180 }

/**
 * The posture a foldable is in, as its button shows pressed: a hinged one's
 * only at a posture's own angle, so a hinge left between them presses none.
 */
export function postureShown(fold: LiveFold | null): Posture | null {
  if (fold?.kind === 'twofold') {
    return fold.open ? 'open' : 'closed'
  }
  if (fold?.kind === 'hinge') {
    return (Object.keys(POSTURE_DEGREES) as Posture[]).find((posture) => POSTURE_DEGREES[posture] === fold.degrees) ?? null
  }
  return null
}

export const DEVICE_CONTROLS: readonly DeviceControl[] = [
  fixed('toolbar.rotateLeft', RotateCcw, (device) => ({ method: 'rotate', params: { device, turn: 'left' } })),
  fixed('toolbar.rotateRight', RotateCw, (device) => ({ method: 'rotate', params: { device, turn: 'right' } })),
  {
    ...fixed('fold.closed', RectangleVertical, (device) => ({ method: 'fold', params: { device, to: 'closed' } }), true),
    pressed: ({ fold }) => postureShown(fold) === 'closed',
    folds: ['hinge', 'twofold'],
  },
  {
    ...fixed('fold.halfOpen', BookOpen, (device) => ({ method: 'fold', params: { device, to: 'half-open' } })),
    pressed: ({ fold }) => postureShown(fold) === 'half-open',
    folds: ['hinge'],
  },
  {
    ...fixed('fold.open', Columns2, (device) => ({ method: 'fold', params: { device, to: 'open' } })),
    pressed: ({ fold }) => postureShown(fold) === 'open',
    folds: ['hinge', 'twofold'],
  },
  fixed('toolbar.appearance', SunMoon, (device) => ({ method: 'toggleAppearance', params: { device } })),
  fixed('toolbar.home', House, (device) => ({ method: 'press', params: { device, button: 'home' } }), true),
  {
    ...fixed('toolbar.back', ArrowLeft, (device) => ({ method: 'press', params: { device, button: 'back' } })),
    platform: 'android',
  },
  fixed('toolbar.lock', Lock, (device) => ({ method: 'press', params: { device, button: 'lock' } })),
  fixed('toolbar.volumeUp', Volume2, (device) => ({ method: 'press', params: { device, button: 'volume-up' } })),
  fixed('toolbar.volumeDown', Volume1, (device) => ({ method: 'press', params: { device, button: 'volume-down' } })),
  fixed('toolbar.screenshot', Camera, (device) => ({ method: 'screenshot', params: { device } }), true),
  {
    label: ({ recording }) => recording ? 'toolbar.stopRecording' : 'toolbar.record',
    icon: ({ recording }) => recording ? Square : Circle,
    call: (device, { recording }) => ({ method: 'record', params: { device, action: recording ? 'stop' : 'start' } }),
    pressed: ({ recording }) => recording,
    alerts: true,
  },
]

/** The controls of a device of `platform` that folds as `fold` says, none of another platform's or fold's own. */
export function deviceControls(platform: MobilePlatform | null, fold: LiveFold['kind'] | null = null): readonly DeviceControl[] {
  return DEVICE_CONTROLS.filter((control) =>
    (control.platform === undefined || control.platform === platform) && (control.folds === undefined || (fold !== null && control.folds.includes(fold))))
}
