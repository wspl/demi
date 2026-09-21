import ipaddr from 'ipaddr.js'

export interface Slot {
  index: number
  hostInterface: string
  peerInterface: string
  namespace: string
  address: string
  gateway: string
}

/** Allocate disjoint /30 networks to active Cloud sandboxes. */
export class SlotPool {
  private readonly taken = new Set<number>()
  constructor(private readonly subnet: string, private readonly count: number) {}

  slot(index: number): Slot {
    const bytes = ipaddr.IPv4.networkAddressFromCIDR(this.subnet).toByteArray()
    const first = bytes.reduce((value, byte) => value * 256 + byte, 0) + index * 4
    const gateway = new ipaddr.IPv4([first >>> 24, first >>> 16 & 255, first >>> 8 & 255, (first & 255) + 1]).toString()
    const address = new ipaddr.IPv4([first >>> 24, first >>> 16 & 255, first >>> 8 & 255, (first & 255) + 2]).toString()
    return { index, hostInterface: `demih${index}`, peerInterface: `demip${index}`, namespace: `demi-${index}`, address, gateway }
  }

  take(): Slot {
    for (let index = 0; index < this.count; index += 1) {
      if (!this.taken.has(index)) {
        this.taken.add(index)
        return this.slot(index)
      }
    }
    throw new Error('All Cloud network slots are in use')
  }

  release(slot: Slot): void {
    this.taken.delete(slot.index)
  }
}
