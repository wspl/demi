import { z } from 'zod'

export const blobKeySchema = z.string().regex(/^[a-f0-9]{64}$/)

/** Content-addressed byte storage for transcript media. */
export interface BlobStore {
  /** Stores bytes and returns their sha256 hex key; putting identical bytes is idempotent. */
  put(data: Uint8Array): Promise<string>
  /** Missing keys return null. Malformed keys, corrupt bytes and IO failures throw. */
  get(sha256: string): Promise<Uint8Array | null>
}
