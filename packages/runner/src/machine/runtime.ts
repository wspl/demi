import type { Host } from '@demicodes/shell'

export const argv = tjs.args
export const env = tjs.env
export const pid = tjs.pid
export const version = tjs.version
export const cwd = (): string => tjs.cwd
export const exit = (code: number): never => tjs.exit(code)
export const onSignal = (signal: tjs.Signal, listener: () => void): (() => void) => {
  tjs.addSignalListener(signal, listener)
  return () => tjs.removeSignalListener(signal, listener)
}

// Read lazily: PID 1 mounts the guest filesystem before consulting passwd.
export const identity: Host['identity'] = {
  get uid() {
    return tjs.system.userInfo.userId
  },
  get gid() {
    return tjs.system.userInfo.groupId
  },
  get hostname() {
    return tjs.hostName
  },
  get homeDir() {
    return tjs.homeDir
  },
}

export function dropPrivileges(uid: number, gid: number): Host['identity'] {
  const user = tjs.dropPrivileges(uid, gid)
  return {
    uid: user.userId,
    gid: user.groupId,
    hostname: tjs.hostName,
    homeDir: user.homeDir ?? '/'
  }
}

export async function fdNode(fd: number): Promise<string | null> {
  try {
    const stat = await tjs.fstat(fd)
    return `${stat.dev}:${stat.ino}`
  } catch (error) {
    if ((error as { code?: string }).code === 'EBADF')
      return null
    throw error
  }
}
