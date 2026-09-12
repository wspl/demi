import { z } from 'zod'

export const NATIVE_PROTOCOL_VERSION = 1
export const NATIVE_TARGETS = [
  'aarch64-apple-darwin',
  'x86_64-apple-darwin',
  'aarch64-unknown-linux-musl',
  'x86_64-unknown-linux-musl',
  'aarch64-pc-windows-msvc',
  'x86_64-pc-windows-msvc',
] as const

export const nativeTargetSchema = z.enum(NATIVE_TARGETS)
export const artifactDigestSchema = z.string().regex(/^[a-f0-9]{64}$/)
export const nativePackageIdSchema = z.string().regex(/^[a-z0-9]+(?:[.-][a-z0-9]+)+$/)
export const nativeArtifactSchema = z.object({
  sha256: artifactDigestSchema,
  size: z.number().int().positive().max(Number.MAX_SAFE_INTEGER),
}).strict()

export const nativePackageSchema = z.object({
  id: nativePackageIdSchema,
  version: z.string().min(1),
  protocolVersion: z.literal(NATIVE_PROTOCOL_VERSION),
  operations: z.array(z.string().min(1)).min(1)
    .refine(operations => new Set(operations).size === operations.length,
      'Operation ids must be unique'),
  targets: z.record(nativeTargetSchema, nativeArtifactSchema),
}).strict()

export type NativeTarget = z.infer<typeof nativeTargetSchema>
export type NativeArtifact = z.infer<typeof nativeArtifactSchema>
export type NativePackage = z.infer<typeof nativePackageSchema>

export const nativeBindingSchema = z.object({
  package: nativePackageIdSchema,
  operation: z.string().min(1),
}).strict()
export type NativeBinding = z.infer<typeof nativeBindingSchema>

export const artifactLocationSchema = z.union([
  z.object({ url: z.url(), expiresAt: z.number().int().optional() }).strict(),
  z.object({ path: z.string().min(1) }).strict(),
])
export type ArtifactLocation = z.infer<typeof artifactLocationSchema>
export type ArtifactResolver = (
  artifact: NativeArtifact,
  signal: AbortSignal,
) => Promise<ArtifactLocation>

/** Deterministic JSON used for manifest and package identities in both languages. */
export function canonicalJson(value: unknown): string {
  if (typeof value === 'string' && /[\uD800-\uDFFF]/u.test(value))
    throw new TypeError('Canonical JSON strings must contain valid Unicode')
  if (value === null || typeof value === 'boolean' || typeof value === 'string')
    return JSON.stringify(value)
  if (typeof value === 'number' && Number.isFinite(value))
    return JSON.stringify(value)
  if (Array.isArray(value))
    return `[${value.map(canonicalJson).join(',')}]`
  if (typeof value === 'object' && value !== null) {
    const object = value as Record<string, unknown>
    return `{${Object.keys(object).sort().map(key => `${canonicalJson(key)}:${canonicalJson(object[key])}`).join(',')}}`
  }
  throw new TypeError('Canonical JSON accepts only JSON values')
}

export async function contentDigest(value: unknown): Promise<string> {
  const bytes = new TextEncoder().encode(canonicalJson(value))
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))
  return Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join('')
}
