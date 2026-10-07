// What every command gets from the daemon, and the helpers they share: their
// options, the addresses they open, and how they fail.
import { parseArgs, type ParseArgsConfig } from 'node:util'
import type { Browser } from './browser'
import type { Network } from './network'
import { local, type Slot } from './slot'

export interface Context {
  slot: Slot
  browser: Browser
  /** The slot's network, which the browser reaches the web app through. */
  network(): Promise<Network>
  /** Prints a line of the command's output on the caller's terminal. */
  print(line: string): void
  /** The caller's `DEMI_*` variables, which the servers it starts get. */
  env: Record<string, string>
  /** Runs another command line, as `timeline` runs its action. */
  run(argv: string[]): Promise<void>
  /** Ends the daemon once the command has answered, as `stop` and `down` do. */
  endDaemon(): void
}

/** A failure the command explains itself: its message is all the caller needs. */
export class CheckFailure extends Error {}

/** The options and positionals of `argv` under `options`, failing with `usage` when they do not fit. */
export function parse<T extends NonNullable<ParseArgsConfig['options']>>(
  argv: string[],
  options: T,
  usage: string,
) {
  try {
    return parseArgs({ args: argv, options, allowPositionals: true, strict: true })
  } catch (error) {
    throw new CheckFailure(`${error instanceof Error ? error.message : String(error)}\nUsage: bun check ${usage}`)
  }
}

/** The number an option or argument gives, or a failure that names it. */
export function numeric(value: string, name: string): number {
  const number = Number(value)
  if (!Number.isFinite(number) || !/^-?\d+(\.\d+)?$/.test(value.trim())) {
    throw new CheckFailure(`${name} must be a number, not ${value}`)
  }
  return number
}

/** The base the browser opens the web app at: the slot's network. */
export function webBase(slot: Slot): string {
  return local(slot.ports.network)
}

/**
 * The URL of an address a command takes: `/chat/c-1` in the web app,
 * `gallery:/session?view=blocks` in the gallery, anything else as written.
 */
export function addressUrl(slot: Slot, address: string): string {
  if (address.startsWith('gallery:')) {
    const path = address.slice('gallery:'.length)
    return local(slot.ports.gallery) + (path.startsWith('/') ? path : `/${path}`)
  }
  if (address.startsWith('/')) {
    return webBase(slot) + address
  }
  return address
}

/** The timeout option every waiting command takes, in seconds. */
export const timeoutOption = { timeout: { type: 'string' } } as const

/** The `--timeout` seconds as milliseconds, or `fallbackSeconds`. */
export function timeoutMs(value: string | undefined, fallbackSeconds: number): number {
  return (value === undefined ? fallbackSeconds : numeric(value, '--timeout')) * 1000
}
