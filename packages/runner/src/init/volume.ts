// A writable filesystem's capacity and explicit sync/grow operations.
import { decodeUtf8 } from '@demicodes/utils'

/** What the runner mode needs of a filesystem; the default over a plain directory syncs and knows nothing else. */
export interface Volume {
  /** Flushes the filesystem to disk. */
  sync(): Promise<void>
  /** The total size the volume should grow to, or null while there is room. */
  wanted(): Promise<number | null>
  /** The backing image is now `bytes` large: grow the filesystem into it. */
  grown(bytes: number): Promise<void>
}

export interface VolumeUsage {
  totalBytes: number
  availableBytes: number
}

/** `df -B1 -P <mount>` as numbers: the filesystem's size and what is left. */
export function parseDf(output: string): VolumeUsage | null {
  const lines = output.trim().split('\n')
  const fields = lines[lines.length - 1]?.trim().split(/\s+/) ?? []
  if (lines.length < 2 || fields.length < 6) return null
  const totalBytes = Number(fields[1])
  const availableBytes = Number(fields[3])
  if (!Number.isFinite(totalBytes) || !Number.isFinite(availableBytes)) return null
  return { totalBytes, availableBytes }
}

export interface GrowthPolicy {
  /** Ask for growth when less than this fraction of the filesystem is free … */
  reserveFraction: number
  /** … or less than this many bytes, for a filesystem large enough that a tenth is too little … */
  reserveBytes: number
  /** … but never more than this fraction, so a small filesystem is not asked to grow at once. */
  reserveCapFraction: number
  /** The size asked for is the current one times this. */
  factor: number
}

export const DEFAULT_GROWTH_POLICY: GrowthPolicy = { reserveFraction: 0.1, reserveBytes: 256 * 1024 * 1024, reserveCapFraction: 0.25, factor: 2 }

/** The reserve: a tenth of the filesystem, raised toward 256 MB but never past a quarter. */
export function reserveBytes(totalBytes: number, policy: GrowthPolicy = DEFAULT_GROWTH_POLICY): number {
  return Math.max(totalBytes * policy.reserveFraction, Math.min(policy.reserveBytes, totalBytes * policy.reserveCapFraction))
}

/** The size to ask for, or null while the reserve holds. */
export function growthWanted(usage: VolumeUsage, policy: GrowthPolicy = DEFAULT_GROWTH_POLICY): number | null {
  if (usage.availableBytes >= reserveBytes(usage.totalBytes, policy)) return null
  return Math.ceil(usage.totalBytes * policy.factor)
}

export interface VolumeIO {
  run(command: string, args: string[]): Promise<{ code: number | null; stdout: Uint8Array }>
}

/** The image behind `device` mounted at `mount`, for PID 1. */
export class BlockVolume implements Volume {
  constructor(
    private readonly io: VolumeIO,
    private readonly device: string,
    private readonly mount: string,
    private readonly policy: GrowthPolicy = DEFAULT_GROWTH_POLICY,
  ) {}

  async sync(): Promise<void> {
    const result = await this.io.run('sync', ['-f', this.mount])
    if (result.code !== 0) throw new Error(`sync ${this.mount} failed`)
  }

  async wanted(): Promise<number | null> {
    const df = await this.io.run('df', ['-B1', '-P', this.mount])
    const usage = df.code === 0 ? parseDf(decodeUtf8(df.stdout)) : null
    return usage ? growthWanted(usage, this.policy) : null
  }

  async grown(bytes: number): Promise<void> {
    const result = await this.io.run('resize2fs', [this.device])
    if (result.code !== 0) throw new Error(`resize2fs ${this.device} exited ${result.code ?? 'by signal'} after growth to ${bytes} bytes`)
  }
}

/** A connected host with no managed block volumes: `sync` flushes everything and nothing else applies. */
export class DirectoryVolume implements Volume {
  constructor(private readonly io: VolumeIO) {}

  async sync(): Promise<void> {
    const result = await this.io.run('sync', [])
    if (result.code !== 0) throw new Error('sync failed')
  }

  async wanted(): Promise<number | null> {
    return null
  }

  async grown(): Promise<void> {}
}
