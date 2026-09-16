import type { ControlService, DeviceRecord, ExposeRecord } from '../storage/control'
import { errorMessage } from '@demicodes/utils'

/** One expose lifetime (`expose.md` § Lifetime): one hour, renewable forever. */
export const EXPOSE_LIFETIME_MS = 60 * 60_000

/** Concurrent relayed connections per expose; the 65th answers 503. */
export const EXPOSE_CONNECTION_LIMIT = 64

/** The expiry sweep's production cadence (`expose.md` § Lifetime). */
const DEFAULT_SWEEP_MS = 60_000

const BASE32_ALPHABET = 'abcdefghijklmnopqrstuvwxyz234567'

/** 128 random bits as 26 lowercase base32 characters — a DNS label and the only credential. */
export function generateExposeId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16))
  let bits = 0
  let acc = 0
  let out = ''
  for (const byte of bytes) {
    acc = (acc << 8) | byte
    bits += 8
    while (bits >= 5) {
      bits -= 5
      out += BASE32_ALPHABET[(acc >> bits) & 31]
    }
  }
  // The final partial character carries the leftover bits: 128 bits is 26
  // base32 characters, not the 25 the whole groups spell.
  if (bits > 0)
    out += BASE32_ALPHABET[(acc << (5 - bits)) & 31]
  return out
}

/**
 * A bare port means `127.0.0.1:<port>`; anything else is used exactly as
 * given — the device's network is the boundary, not Demi.
 */
export function normalizeExposeAddress(address: string): string {
  return /^\d+$/.test(address) ? `127.0.0.1:${address}` : address
}

/** Split a stored address into the runner's `net_open` host and port. */
export function exposeTarget(address: string): { host: string; port: number } {
  const index = address.lastIndexOf(':')
  if (index === -1)
    throw new Error(`expose address ${address} has no port`)
  const port = Number(address.slice(index + 1))
  if (!Number.isInteger(port) || port < 1 || port > 65535)
    throw new Error(`expose address ${address} has no valid port`)
  return { host: address.slice(0, index), port }
}

/** The error codes `expose.md` § Commands and `web-api.md` § Exposes define. */
export class ExposeError extends Error {
  constructor(
    readonly code: 'expose_unavailable' | 'device_offline' | 'expose_not_found' | 'device_not_found',
    message: string
  ) {
    super(message)
    this.name = 'ExposeError'
  }
}

/** The `host:port` of the service and the hostname that reaches it. */
export function exposeHostname(id: string, domain: string): string {
  return `${id}.${domain}`
}

/** `expose.md` § Deployment: the scheme of the printed URLs follows the forwarded protocol. */
export function exposeUrl(
  id: string,
  domain: string,
  scheme: string
): string {
  return `${scheme}://${exposeHostname(id, domain)}/`
}

/** The response shape of an expose on every surface (`web-api.md` § Exposes). */
export interface ExposeView {
  id: string
  deviceId: string
  address: string
  url: string
  createdAt: string
  expiresAt: string
}

/**
 * The expose records and their one-hour lifetime (`expose.md`). Owns the
 * clock, the expiry sweep and the registry of live relayed connections: every
 * path that destroys a record — expiry, remove, Cloud stop, revocation —
 * ends its connections through here.
 */
export class Exposes {
  private readonly sweepTimer: ReturnType<typeof setInterval> | null = null
  private readonly connections = new Map<string, Set<() => void>>()

  constructor(private readonly deps: {
    control: ControlService
    /** `DEMI_EXPOSE_DOMAIN`; null disables the whole feature. */
    domain: string | null
    /** The registry's live-connection check: creation requires a connected device. */
    deviceOnline: (deviceId: string) => boolean
    now: () => number
    /** The fallback scheme of printed URLs when no request supplies one. */
    scheme: () => string
    /** Tests raise the sweep cadence; production sweeps every 60 s. */
    sweepMs?: number
    log?: (line: string) => void
  }) {
    this.sweepTimer = setInterval(
      () => void this.sweep().catch(error =>
        this.deps.log?.(`expose sweep: ${errorMessage(error)}`)
      ),
      deps.sweepMs ?? DEFAULT_SWEEP_MS
    )
    // The sweep is housekeeping: it must not hold the process open.
    this.sweepTimer.unref()
  }

  get domain(): string | null {
    return this.deps.domain
  }

  close(): void {
    if (this.sweepTimer !== null)
      clearInterval(this.sweepTimer)
    for (const close of [...this.connections.values()].flatMap(set => [...set]))
      close()
    this.connections.clear()
  }

  /** Creation: one hour from now, on a connected caller-owned device. */
  async add(
    userId: string,
    device: DeviceRecord,
    address: string
  ): Promise<ExposeRecord> {
    if (this.deps.domain === null)
      throw new ExposeError(
        'expose_unavailable',
        'This backend has no expose domain configured (DEMI_EXPOSE_DOMAIN)'
      )
    if (!this.deps.deviceOnline(device.id))
      throw new ExposeError(
        'device_offline',
        `The device ${device.id} is offline; connect it before exposing a service`
      )
    const now = this.deps.now()
    return this.deps.control.createExpose({
      id: generateExposeId(),
      userId,
      deviceId: device.id,
      address: normalizeExposeAddress(address),
      createdAt: new Date(now).toISOString(),
      expiresAt: new Date(now + EXPOSE_LIFETIME_MS).toISOString(),
    })
  }

  /** The user's live exposes, soonest expiry first; expired rows die here too. */
  async list(userId: string): Promise<ExposeRecord[]> {
    await this.sweep()
    return (await this.deps.control.listExposes(userId))
      .filter(record => !this.expired(record))
  }

  async renew(userId: string, id: string): Promise<ExposeRecord> {
    await this.sweep()
    const record = await this.deps.control.getExpose(id)
    if (!record || record.userId !== userId || this.expired(record))
      throw new ExposeError('expose_not_found', `No expose ${id}`)
    const expiresAt = new Date(
      this.deps.now() + EXPOSE_LIFETIME_MS
    ).toISOString()
    return (await this.deps.control.renewExpose(id, userId, expiresAt))!
  }

  async remove(userId: string, id: string): Promise<void> {
    const record = await this.deps.control.getExpose(id)
    if (!record || record.userId !== userId || this.expired(record))
      throw new ExposeError('expose_not_found', `No expose ${id}`)
    await this.destroy(id)
  }

  /**
   * The record behind a hostname, for the relay only: null when unknown, and
   * destroyed-and-null once expired.
   */
  async liveRecord(id: string): Promise<ExposeRecord | null> {
    const record = await this.deps.control.getExpose(id)
    if (!record)
      return null
    if (this.expired(record)) {
      await this.destroy(id)
      return null
    }
    return record
  }

  /** Deletes expired rows and ends their connections. */
  async sweep(): Promise<void> {
    const destroyed = await this.deps.control.deleteExpiredExposes(
      new Date(this.deps.now()).toISOString()
    )
    for (const id of destroyed)
      this.endConnections(id)
  }

  /** Every expose on a device: a Cloud stop and device revocation. */
  async destroyForDevice(deviceId: string): Promise<void> {
    for (const id of await this.deps.control.deleteExposesByDevice(deviceId))
      this.endConnections(id)
  }

  /** Destroys one record at once; its URL stops working. */
  private async destroy(id: string): Promise<void> {
    await this.deps.control.deleteExpose(id)
    this.endConnections(id)
  }

  private expired(record: ExposeRecord): boolean {
    return Date.parse(record.expiresAt) <= this.deps.now()
  }

  url(record: ExposeRecord, scheme?: string): string {
    return exposeUrl(
      record.id,
      this.deps.domain!,
      scheme ?? this.deps.scheme()
    )
  }

  /** The response shape every surface prints (`web-api.md` § Exposes). */
  view(record: ExposeRecord): ExposeView {
    return {
      id: record.id,
      deviceId: record.deviceId,
      address: record.address,
      url: this.url(record),
      createdAt: record.createdAt,
      expiresAt: record.expiresAt,
    }
  }

  /**
   * Registers one relayed connection on an expose; the closer ends it from
   * either side. Returns null at the concurrent-connection limit.
   */
  begin(id: string, close: () => void): (() => void) | null {
    let live = this.connections.get(id)
    if (!live) {
      live = new Set()
      this.connections.set(id, live)
    }
    if (live.size >= EXPOSE_CONNECTION_LIMIT)
      return null
    live.add(close)
    let released = false
    return () => {
      if (released)
        return
      released = true
      live!.delete(close)
      if (live!.size === 0)
        this.connections.delete(id)
    }
  }

  private endConnections(id: string): void {
    const live = this.connections.get(id)
    if (!live)
      return
    this.connections.delete(id)
    for (const close of [...live])
      close()
  }
}
