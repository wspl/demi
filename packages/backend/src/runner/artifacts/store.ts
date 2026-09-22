import type { NativeArtifact } from '@demicodes/command-protocol'

export interface ObjectSource extends NativeArtifact {
  body: string | Buffer
}

/** Storage belongs to the backend; runner responses contain only signed locations. */
export interface ArtifactStore {
  putImmutable(key: string, source: ObjectSource, signal: AbortSignal): Promise<void>
  signGet(key: string, expiresIn: number, signal: AbortSignal): Promise<string>
  close(): void
}
