/**
 * The device toolbar's controls (`mobile.md` § The live view), in their
 * order: each the page method it calls and that method's parameters, which
 * run the operation of the `demi mobile` command that does the same, so
 * Rotate Left is `demi mobile rotate <m> left` and Lock is
 * `demi mobile key <m> lock`. The toolbar is drawn from this table and the
 * controller runs its calls.
 */
import type { Component } from 'vue'
import { Camera, Circle, House, Lock, RotateCcw, RotateCw, Square, SunMoon, Volume1, Volume2 } from '@lucide/vue'
import type { OnDevice, PressButton, RecordDevice, RotateDevice } from '../generated/plugin'

/** A page method's call: its name and parameters. */
export type DeviceCall =
  | { method: 'rotate'; params: RotateDevice }
  | { method: 'toggleAppearance'; params: OnDevice }
  | { method: 'press'; params: PressButton }
  | { method: 'screenshot'; params: OnDevice }
  | { method: 'record'; params: RecordDevice }

/** The toolbar's names, keys of the page's text. */
export type ControlLabel =
  | 'toolbar.rotateLeft'
  | 'toolbar.rotateRight'
  | 'toolbar.appearance'
  | 'toolbar.home'
  | 'toolbar.lock'
  | 'toolbar.volumeUp'
  | 'toolbar.volumeDown'
  | 'toolbar.screenshot'
  | 'toolbar.record'
  | 'toolbar.stopRecording'

/** A device's state the toolbar shows. */
export interface ControlState {
  recording: boolean
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
}

function fixed(label: ControlLabel, icon: Component, call: (device: string) => DeviceCall, group = false): DeviceControl {
  return { label: () => label, icon: () => icon, call, group }
}

export const DEVICE_CONTROLS: readonly DeviceControl[] = [
  fixed('toolbar.rotateLeft', RotateCcw, (device) => ({ method: 'rotate', params: { device, turn: 'left' } })),
  fixed('toolbar.rotateRight', RotateCw, (device) => ({ method: 'rotate', params: { device, turn: 'right' } })),
  fixed('toolbar.appearance', SunMoon, (device) => ({ method: 'toggleAppearance', params: { device } })),
  fixed('toolbar.home', House, (device) => ({ method: 'press', params: { device, button: 'home' } }), true),
  fixed('toolbar.lock', Lock, (device) => ({ method: 'press', params: { device, button: 'lock' } })),
  fixed('toolbar.volumeUp', Volume2, (device) => ({ method: 'press', params: { device, button: 'volume-up' } })),
  fixed('toolbar.volumeDown', Volume1, (device) => ({ method: 'press', params: { device, button: 'volume-down' } })),
  fixed('toolbar.screenshot', Camera, (device) => ({ method: 'screenshot', params: { device } }), true),
  {
    label: ({ recording }) => recording ? 'toolbar.stopRecording' : 'toolbar.record',
    icon: ({ recording }) => recording ? Square : Circle,
    call: (device, { recording }) => ({ method: 'record', params: { device, action: recording ? 'stop' : 'start' } }),
    pressed: ({ recording }) => recording,
  },
]
