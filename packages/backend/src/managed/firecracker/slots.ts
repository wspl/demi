// VM slots (`managed-hosts.md` § Provisioning): the tap pool the install
// script created, one tap per slot with a /30 out of the managed subnet —
// the host at the first address, the guest at the second — and, in jailer
// mode, one uid per slot. Slots are taken by running VMs and given back
// when they die or are killed.

export interface Slot {
  index: number
  tap: string
  /** The guest's address with the /30 prefix, as the kernel command line carries it. */
  guestAddress: string
  gateway: string
  mac: string
}

export interface SlotPoolOptions {
  /** The managed subnet, `172.16.0.0/16`; each slot takes a /30 out of it. */
  subnet: string
  /** How many slots the install script created. */
  count: number
  /** The tap name prefix; slot `n` is `<prefix><n>`. */
  tapPrefix: string
}

/** The /30 of slot `index` inside `subnet`: host `.1`, guest `.2`. */
export function slotOf(options: SlotPoolOptions, index: number): Slot {
  const [network, prefix] = options.subnet.split('/')
  const bits = Number(prefix)
  const base = ipToNumber(network!)
  if (!Number.isFinite(bits) || bits > 30) throw new Error(`managed subnet ${options.subnet} is not a network of /30s`)
  if ((index + 1) * 4 > 2 ** (32 - bits)) throw new Error(`slot ${index} does not fit in ${options.subnet}`)
  const first = base + index * 4
  return {
    index,
    tap: `${options.tapPrefix}${index}`,
    guestAddress: `${numberToIp(first + 2)}/30`,
    gateway: numberToIp(first + 1),
    // Locally administered, unicast; the slot in the low bytes.
    mac: `06:fc:${hex(index >> 24)}:${hex(index >> 16)}:${hex(index >> 8)}:${hex(index)}`,
  }
}

export class SlotPool {
  private readonly taken = new Set<number>()

  constructor(private readonly options: SlotPoolOptions) {}

  take(): Slot {
    for (let index = 0; index < this.options.count; index += 1) {
      if (this.taken.has(index)) continue
      this.taken.add(index)
      return slotOf(this.options, index)
    }
    throw new Error(`all ${this.options.count} VM slots are in use`)
  }

  release(slot: Slot): void {
    this.taken.delete(slot.index)
  }

  get free(): number {
    return this.options.count - this.taken.size
  }
}

function ipToNumber(ip: string): number {
  const parts = ip.split('.').map(Number)
  if (parts.length !== 4 || parts.some((part) => !Number.isInteger(part) || part < 0 || part > 255)) throw new Error(`not an IPv4 address: ${ip}`)
  return ((parts[0]! << 24) >>> 0) + (parts[1]! << 16) + (parts[2]! << 8) + parts[3]!
}

function numberToIp(value: number): string {
  return [value >>> 24, (value >>> 16) & 255, (value >>> 8) & 255, value & 255].join('.')
}

function hex(byte: number): string {
  return (byte & 255).toString(16).padStart(2, '0')
}
