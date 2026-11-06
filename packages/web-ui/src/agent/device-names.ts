import { inject, provide, type InjectionKey } from 'vue'
import type { TargetKind } from '../targets/types'

/** A Device as the page names it, and which kind it is. */
export interface DeviceName {
  name: string
  kind: TargetKind
}

/**
 * What the page calls the device whose device is `device`: the Cloud, or a
 * paired device by its name.
 */
export type DeviceNames = (device: string) => DeviceName

const deviceNamesKey: InjectionKey<() => DeviceNames | undefined> = Symbol('device-names')

/**
 * Offers the records below what each device is called, read at each render, so
 * a subagent that works on another device than the conversation's names it;
 * none where the host does not know its devices, and the device's id then
 * stands for a device's name.
 */
export function provideDeviceNames(source: () => DeviceNames | undefined): void {
  provide(deviceNamesKey, source)
}

export function useDeviceNames(): () => DeviceNames | undefined {
  return inject(deviceNamesKey, () => undefined)
}
