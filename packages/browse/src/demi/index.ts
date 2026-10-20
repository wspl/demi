// `demi`, the project's helpers a script gets (browse.md § What `demi`
// adds): one file each in this folder, bound here to the call they run in.
import type { Locator } from 'playwright'
import type { Tool } from '../tool'
import { down, type DownArgument } from './down'
import { emulate } from './emulate'
import { focusRings } from './focus-rings'
import { frames } from './frames'
import { gallery } from './gallery'
import { grant } from './grant'
import { help } from './help'
import { hide, show, type VisibilityOptions } from './hide'
import { ime } from './ime'
import { log } from './log'
import { message } from './message'
import { net } from './net'
import { pixel } from './pixel'
import { runner } from './runner'
import { shot } from './shot'
import { stop } from './stop'
import { timeline } from './timeline'
import { turn } from './turn'
import { up, type UpOptions } from './up'
import { webkitPage } from './webkit'

export function createDemi(tool: Tool) {
  return {
    up: (...args: (string | UpOptions)[]) => up(tool, args),
    down: (...args: DownArgument[]) => down(tool, args),
    stop: () => stop(tool),
    help: () => help(tool),
    shot: (name?: string, options?: Parameters<typeof shot>[2]) => shot(tool, name, options),
    timeline: (action: () => unknown, moments: number[], options?: Parameters<typeof timeline>[3]) => timeline(tool, action, moments, options),
    frames: <T>(probe: () => T) => frames(tool, probe),
    focusRings: Object.assign(
      (options?: Parameters<typeof focusRings>[1]) => focusRings(tool, options),
      { of: (locator: Locator, options?: Parameters<typeof focusRings>[1]) => focusRings.of(tool, locator, options) },
    ),
    pixel: (x: number, y: number) => pixel(tool, x, y),
    net: net(tool),
    ime: (text: string, options?: Parameters<typeof ime>[2]) => ime(tool, text, options),
    webkit: () => webkitPage(tool),
    emulate: (options: Parameters<typeof emulate>[1]) => emulate(tool, options),
    hide: (options?: VisibilityOptions) => hide(tool, options),
    show: (options?: VisibilityOptions) => show(tool, options),
    grant: (names: string | string[], options?: { origin?: string }) => grant(tool, names, options),
    gallery: (path?: string) => gallery(tool, path),
    message: (text: string, options?: Parameters<typeof message>[2]) => message(tool, text, options),
    turn: (options?: { timeout?: number }) => turn(tool, options),
    runner: runner(tool),
    log: log(tool),
  }
}

export type Demi = ReturnType<typeof createDemi>
