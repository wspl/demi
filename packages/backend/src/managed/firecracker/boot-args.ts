// The kernel command line of a guest (`managed-hosts.md` § Joining): the
// kernel's own parameters, then the `demi.*` parameters the runner's init
// reads (`packages/runner/src/init/cmdline.ts`).
import type { Slot } from './slots'

export interface GuestBootArgs {
  backendUrl: string
  deviceToken: string
  slot: Slot
  dns: string[]
  /** The home image was just made from a directory: the guest chowns it to its user on this boot. */
  firstBoot: boolean
}

/** Firecracker's recommended kernel parameters: serial console, exit on panic, no PCI scan, the runner as init. */
export const KERNEL_ARGS = 'console=ttyS0 reboot=k panic=1 pci=off init=/demi-runner'

export function bootArgs(args: GuestBootArgs): string {
  for (const [name, value] of [
    ['backend URL', args.backendUrl],
    ['device token', args.deviceToken],
  ]) {
    if (/\s|"/.test(value!)) throw new Error(`the ${name} cannot ride the kernel command line: ${value}`)
  }
  const parts = [
    KERNEL_ARGS,
    `demi.backend=${args.backendUrl}`,
    `demi.token=${args.deviceToken}`,
    `demi.ip=${args.slot.guestAddress}`,
    `demi.gw=${args.slot.gateway}`,
  ]
  if (args.dns.length > 0) parts.push(`demi.dns=${args.dns.join(',')}`)
  if (args.firstBoot) parts.push('demi.firstboot=1')
  return parts.join(' ')
}
