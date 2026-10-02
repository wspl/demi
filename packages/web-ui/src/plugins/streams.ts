/**
 * A plugin's user stream as a page holds it (`web-api.md` § User streams):
 * bytes both ways between the page and the plugin's package on the
 * conversation's Host, over the product's WebSocket or a gallery's fixture.
 * What the bytes mean is the plugin's own protocol.
 */

/** Bytes the page sends: their own buffer, as a socket requires. */
export type StreamBytes = Uint8Array<ArrayBuffer>

/** One open stream. */
export interface UserStream {
  send(bytes: StreamBytes): void
  close(): void
}

export interface UserStreamHandlers {
  data(bytes: Uint8Array): void
  /** The stream ended; `reason` is the close reason the backend gave. */
  closed(reason: string): void
}

/** Opens one stream, whose bytes reach `handlers`. */
export type OpenUserStream = (handlers: UserStreamHandlers) => UserStream
