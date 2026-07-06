import { concatBytes, decodeUtf8 } from '@demicodes/utils'
import type { HostSpawnRedirection, ShellOptions, ShoptOptions } from '@demicodes/just-bash/interpreter'
import type { ForegroundProcess, ForegroundSink, ShellSession } from './environment-state'
import type { HostProcessOutputChunk } from './host'
import type { HostBackedFileSystem } from './host-fs'

export function createOutputSinks(
  fs: HostBackedFileSystem,
  cwd: string,
  redirections: HostSpawnRedirection[] | undefined,
): Record<1 | 2, ForegroundSink> {
  const routes: Record<1 | 2, ForegroundSink> = {
    1: visibleSink(1),
    2: visibleSink(2),
  }

  for (const redirection of redirections ?? []) {
    switch (redirection.operator) {
      case '>':
      case '>|': {
        routes[(redirection.fd ?? 1) as 1 | 2] = targetSink(fs, cwd, redirection.target, false)
        break
      }
      case '>>': {
        routes[(redirection.fd ?? 1) as 1 | 2] = targetSink(fs, cwd, redirection.target, true)
        break
      }
      case '&>': {
        const sink = targetSink(fs, cwd, redirection.target, false)
        routes[1] = sink
        routes[2] = sink
        break
      }
      case '&>>': {
        const sink = targetSink(fs, cwd, redirection.target, true)
        routes[1] = sink
        routes[2] = sink
        break
      }
      case '>&': {
        const fd = (redirection.fd ?? 1) as 1 | 2
        if (redirection.target === '-') {
          routes[fd] = nullSink()
          break
        }
        const sourceFd = Number.parseInt(redirection.target.replace(/^&/, ''), 10)
        if (sourceFd === 1 || sourceFd === 2) {
          routes[fd] = routes[sourceFd]
          break
        }
        const sink = targetSink(fs, cwd, redirection.target, false)
        if (redirection.fd === null) {
          routes[1] = sink
          routes[2] = sink
        } else {
          routes[fd] = sink
        }
        break
      }
    }
  }

  return routes
}

export function recordForegroundChunk(foreground: ForegroundProcess, sourceFd: 1 | 2, chunk: Uint8Array): void {
  const text = decodeUtf8(chunk)
  foreground.lastOutputAt = Date.now()
  if (sourceFd === 1) foreground.rawStdoutBuffer += text
  else foreground.rawStderrBuffer += text

  const sink = foreground.outputSinks[sourceFd]
  if (sink.kind === 'file' || sink.kind === 'null') {
    sink.bytes.push(chunk)
    return
  }

  appendVisibleChunk(foreground, sink.fd ?? sourceFd, text, chunk.byteLength)
}

export async function flushForegroundSinks(session: ShellSession, foreground: ForegroundProcess): Promise<void> {
  const flushed = new Set<ForegroundSink>()
  for (const sink of [foreground.outputSinks[1], foreground.outputSinks[2]]) {
    if (sink.kind !== 'file' || flushed.has(sink)) continue
    flushed.add(sink)
    const bytes = concatBytes(sink.bytes)
    if (sink.append) await session.fs.appendFile(sink.path as string, bytes)
    else await session.fs.writeFile(sink.path as string, bytes)
  }
}

export function notifyForegroundWaiters(
  waiters: Set<(foreground: ForegroundProcess) => void>,
  foreground: ForegroundProcess,
): void {
  const snapshot = [...waiters]
  waiters.clear()
  for (const waiter of snapshot) waiter(foreground)
}

export async function pumpStream(stream: AsyncIterable<Uint8Array>, onChunk: (chunk: Uint8Array) => void): Promise<void> {
  try {
    for await (const chunk of stream) onChunk(chunk)
  } catch {
    // stream errors surface in handle.wait()
  }
}

export async function pumpOutputStream(
  stream: AsyncIterable<HostProcessOutputChunk>,
  onChunk: (chunk: HostProcessOutputChunk) => void,
): Promise<void> {
  try {
    for await (const chunk of stream) onChunk(chunk)
  } catch {
    // stream errors surface in handle.wait()
  }
}

export function buildShellopts(options: ShellOptions): string {
  const flags: string[] = []
  if (options.errexit) flags.push('errexit')
  if (options.pipefail) flags.push('pipefail')
  if (options.nounset) flags.push('nounset')
  if (options.xtrace) flags.push('xtrace')
  if (options.verbose) flags.push('verbose')
  if (options.posix) flags.push('posix')
  if (options.allexport) flags.push('allexport')
  if (options.noclobber) flags.push('noclobber')
  if (options.noglob) flags.push('noglob')
  if (options.noexec) flags.push('noexec')
  if (options.vi) flags.push('vi')
  if (options.emacs) flags.push('emacs')
  return flags.join(':')
}

export function buildBashopts(options: ShoptOptions): string {
  const flags: string[] = []
  if (options.extglob) flags.push('extglob')
  if (options.dotglob) flags.push('dotglob')
  if (options.nullglob) flags.push('nullglob')
  if (options.failglob) flags.push('failglob')
  if (options.globstar) flags.push('globstar')
  if (options.globskipdots) flags.push('globskipdots')
  if (options.nocaseglob) flags.push('nocaseglob')
  if (options.nocasematch) flags.push('nocasematch')
  if (options.expand_aliases) flags.push('expand_aliases')
  if (options.lastpipe) flags.push('lastpipe')
  if (options.xpg_echo) flags.push('xpg_echo')
  return flags.join(':')
}

function visibleSink(fd: 1 | 2): ForegroundSink {
  return { kind: 'visible', fd, bytes: [] }
}

function nullSink(): ForegroundSink {
  return { kind: 'null', bytes: [] }
}

function targetSink(fs: HostBackedFileSystem, cwd: string, target: string, append: boolean): ForegroundSink {
  if (target === '/dev/stdout') return visibleSink(1)
  if (target === '/dev/stderr') return visibleSink(2)
  if (target === '/dev/null') return nullSink()
  return { kind: 'file', path: fs.resolvePath(cwd, target), append, bytes: [] }
}

function appendVisibleChunk(foreground: ForegroundProcess, targetFd: 1 | 2, text: string, byteLength: number): void {
  foreground.outputChunks.push({
    stream: targetFd === 1 ? 'stdout' : 'stderr',
    text,
    offset: foreground.outputBytes,
    bytes: byteLength,
  })
  foreground.outputBytes += byteLength

  if (targetFd === 1) foreground.stdoutBuffer += text
  else foreground.stderrBuffer += text
}
