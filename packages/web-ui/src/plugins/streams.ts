/**
 * A plugin's user stream as a page holds it (`web-api-hosts.md` § User streams):
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

/** How a stream is opened. */
export interface UserStreamOptions {
  /**
   * The stream is a live view's, which the direct path carries on its live
   * peer, its pictures on a channel of their own (`live-view.md` § Two
   * paths, one protocol); only `web-ui`'s live view says so.
   */
  live?: boolean
}

/** Opens one stream, whose bytes reach `handlers`. */
export type OpenUserStream = (handlers: UserStreamHandlers, options?: UserStreamOptions) => UserStream
