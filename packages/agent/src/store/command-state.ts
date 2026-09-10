import { z } from 'zod'
import type { Block } from '@demicodes/core'
import type { PortableJsonValue } from '@demicodes/utils'
import { portableJsonValueSchema } from '../protocol/schemas'

export const commandStorageKeySchema = z.string().min(1).refine(
  (key) => !key.includes('\0') && !key.startsWith('/')
    && !/^[A-Za-z]:[\\/]/.test(key)
    && !key.split(/[\\/]+/).includes('..'),
  'Command storage keys must be relative and contain no path traversal',
)

export const commandValuesSchema = z.record(commandStorageKeySchema, portableJsonValueSchema)

const revisionSchema = z.number().int().nonnegative().safe()

export const commandVersionSchema = z.strictObject({
  revision: revisionSchema,
  values: commandValuesSchema,
})

export const sessionBoundarySchema = z.strictObject({
  blockId: z.string().min(1),
  edge: z.enum(['before_user', 'after_assistant', 'after_block']),
  commandRevision: revisionSchema,
})

export const commandStateSchema = z.strictObject({
  revision: revisionSchema,
  versions: z.array(commandVersionSchema).min(1),
  boundaries: z.array(sessionBoundarySchema),
}).superRefine((state, ctx) => {
  const versions = new Map(state.versions.map((version) => [version.revision, version]))
  const boundaries = new Set(state.boundaries.map((boundary) =>
    `${boundary.edge}\0${boundary.blockId}`,
  ))
  if (versions.size !== state.versions.length
    || !versions.has(state.revision)
    || !versions.has(0)
    || Object.keys(versions.get(0)!.values).length !== 0
    || boundaries.size !== state.boundaries.length
    || state.boundaries.some((boundary) => !versions.has(boundary.commandRevision))) {
    ctx.addIssue({ code: 'custom', message: 'Invalid command-state version or boundary references' })
  }
})

export type CommandValues = z.infer<typeof commandValuesSchema>
export type CommandVersion = z.infer<typeof commandVersionSchema>
export type SessionBoundary = z.infer<typeof sessionBoundarySchema>
export type CommandStateSnapshot = z.infer<typeof commandStateSchema>

export function emptyCommandState(): CommandStateSnapshot {
  return { revision: 0, versions: [{ revision: 0, values: {} }], boundaries: [] }
}

/** Immutable values and mutable cutoff references for one node's journal. */
export class CommandStateHistory {
  private readonly versions: Map<number, CommandVersion>
  private readonly boundaries = new Map<string, SessionBoundary>()
  private currentRevision: number
  private dirty = false

  constructor(snapshot: CommandStateSnapshot = emptyCommandState()) {
    const parsed = commandStateSchema.parse(snapshot)
    this.versions = new Map(parsed.versions.map((version) => [version.revision, structuredClone(version)]))
    this.currentRevision = parsed.revision
    for (const boundary of parsed.boundaries) {
      this.boundaries.set(this.key(boundary.blockId, boundary.edge), { ...boundary })
    }
  }

  get revision(): number {
    return this.currentRevision
  }

  values(): CommandValues {
    return structuredClone(this.versions.get(this.currentRevision)!.values)
  }

  prepare(values: unknown): CommandVersion | null {
    const next = commandValuesSchema.parse(values)
    if (equalValues(next, this.versions.get(this.currentRevision)!.values)) {
      return null
    }
    let revision = 1
    for (const existing of this.versions.keys()) {
      revision = Math.max(revision, existing + 1)
    }
    revisionSchema.parse(revision)
    return { revision, values: structuredClone(next) }
  }

  /** Adopt only the committed version; cutoffs captured during IO stay intact. */
  accept(version: CommandVersion): void {
    this.versions.set(version.revision, structuredClone(version))
    this.currentRevision = version.revision
  }

  capture(blockId: string, edge: SessionBoundary['edge'], revision = this.currentRevision): void {
    if (!this.versions.has(revision)) {
      throw new Error(`Unknown command-state revision ${revision}`)
    }
    const key = this.key(blockId, edge)
    const previous = this.boundaries.get(key)
    if (previous?.commandRevision === revision) {
      return
    }
    if (previous && edge !== 'after_block') {
      throw new Error(`The ${edge} boundary for ${blockId} is already recorded`)
    }
    this.boundaries.set(key, { blockId, edge, commandRevision: revision })
    this.dirty = true
  }

  boundary(blockId: string, edge: SessionBoundary['edge']): number {
    const boundary = this.boundaries.get(this.key(blockId, edge))
    if (!boundary) {
      throw new Error(`No ${edge} command-state boundary for message ${blockId}`)
    }
    return boundary.commandRevision
  }

  hasBoundary(blockId: string, edge: SessionBoundary['edge']): boolean {
    return this.boundaries.has(this.key(blockId, edge))
  }

  /** Restore keeps all immutable versions; only retained block references survive. */
  select(blocks: readonly Block[], revision: number, copyReferencedOnly = false): CommandStateSnapshot {
    const retained = new Set(blocks.map((block) => block.id))
    const boundaries = [...this.boundaries.values()].filter((boundary) => retained.has(boundary.blockId))
    const referenced = new Set([0, revision, ...boundaries.map((boundary) => boundary.commandRevision)])
    const versions = [...this.versions.values()].filter((version) =>
      !copyReferencedOnly || referenced.has(version.revision),
    )
    return commandStateSchema.parse(structuredClone({ revision, versions, boundaries }))
  }

  retainBoundaries(blocks: readonly Block[]): void {
    const retained = new Set(blocks.map((block) => block.id))
    for (const [key, boundary] of this.boundaries) {
      if (!retained.has(boundary.blockId)) {
        this.boundaries.delete(key)
        this.dirty = true
      }
    }
  }

  snapshot(pending?: CommandVersion): CommandStateSnapshot {
    return structuredClone({
      revision: pending?.revision ?? this.currentRevision,
      versions: [...this.versions.values(), ...(pending ? [pending] : [])],
      boundaries: [...this.boundaries.values()],
    })
  }

  takeUpdate(pending?: CommandVersion): CommandStateSnapshot | undefined {
    if (!this.dirty && !pending) {
      return undefined
    }
    this.dirty = false
    return this.snapshot(pending)
  }

  markDirty(): void {
    this.dirty = true
  }

  private key(blockId: string, edge: SessionBoundary['edge']): string {
    return `${edge}\0${blockId}`
  }
}

function equalValues(left: PortableJsonValue, right: PortableJsonValue): boolean {
  if (left === right) {
    return true
  }
  if (left === null || right === null || typeof left !== 'object' || typeof right !== 'object') {
    return false
  }
  if (left instanceof Date || right instanceof Date) {
    return left instanceof Date && right instanceof Date && left.getTime() === right.getTime()
  }
  if (left instanceof Uint8Array || right instanceof Uint8Array) {
    return left instanceof Uint8Array && right instanceof Uint8Array
      && left.length === right.length && left.every((byte, index) => byte === right[index])
  }
  if (Array.isArray(left) || Array.isArray(right)) {
    return Array.isArray(left) && Array.isArray(right)
      && left.length === right.length && left.every((value, index) => equalValues(value, right[index]))
  }
  const first = left as Record<string, PortableJsonValue>
  const second = right as Record<string, PortableJsonValue>
  const keys = Object.keys(first)
  return keys.length === Object.keys(second).length
    && keys.every((key) => Object.hasOwn(second, key) && equalValues(first[key]!, second[key]!))
}
