// `demi`, the project's helpers a script gets (browse.md § What `demi`
// adds): one file each in this folder, bound here to the call they run in.
import type { Tool } from '../tool'
import { down, type DownArgument } from './down'
import { emulate } from './emulate'
import { gallery } from './gallery'
import { grant } from './grant'
import { help } from './help'
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
import { up } from './up'

export function createDemi(tool: Tool) {
  return {
    up: (...servers: string[]) => up(tool, servers),
    down: (...args: DownArgument[]) => down(tool, args),
    stop: () => stop(tool),
    help: () => help(tool),
    shot: (name?: string, options?: Parameters<typeof shot>[2]) => shot(tool, name, options),
    timeline: (action: () => unknown, moments: number[], options?: Parameters<typeof timeline>[3]) => timeline(tool, action, moments, options),
    pixel: (x: number, y: number) => pixel(tool, x, y),
    net: net(tool),
    ime: (text: string, options?: Parameters<typeof ime>[2]) => ime(tool, text, options),
    emulate: (options: Parameters<typeof emulate>[1]) => emulate(tool, options),
    grant: (names: string | string[], options?: { origin?: string }) => grant(tool, names, options),
    gallery: (path?: string) => gallery(tool, path),
    message: (text: string, options?: Parameters<typeof message>[2]) => message(tool, text, options),
    turn: (options?: { timeout?: number }) => turn(tool, options),
    runner: runner(tool),
    log: log(tool),
  }
}

export type Demi = ReturnType<typeof createDemi>
