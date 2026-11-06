import { inject, provide, type InjectionKey } from 'vue'
import type { HostKind } from '../hosts/types'

/** A Host as the page names it, and which kind it is. */
export interface HostName {
  name: string
  kind: HostKind
}

/**
 * What the page calls the Host whose device is `device`: the Cloud, or a
 * paired device by its name.
 */
export type HostNames = (device: string) => HostName

const hostNamesKey: InjectionKey<() => HostNames | undefined> = Symbol('host-names')

/**
 * Offers the records below what each Host is called, read at each render, so
 * a subagent that works on another Host than the conversation's names it;
 * none where the host does not know its devices, and the device's id then
 * stands for a device's name.
 */
export function provideHostNames(source: () => HostNames | undefined): void {
  provide(hostNamesKey, source)
}

export function useHostNames(): () => HostNames | undefined {
  return inject(hostNamesKey, () => undefined)
}
