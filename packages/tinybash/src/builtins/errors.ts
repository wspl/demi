import { errorCode, errorMessage } from '@demicodes/utils'

/** GNU `strerror` texts for the codes a Host filesystem raises. */
const STRERROR: Record<string, string> = {
  ENOENT: 'No such file or directory',
  EISDIR: 'Is a directory',
  ENOTDIR: 'Not a directory',
  EEXIST: 'File exists',
  EACCES: 'Permission denied',
  EPERM: 'Operation not permitted',
  ENOTEMPTY: 'Directory not empty',
  ENOSPC: 'No space left on device',
  EINVAL: 'Invalid argument',
  ELOOP: 'Too many levels of symbolic links',
  ENAMETOOLONG: 'File name too long',
  EXDEV: 'Invalid cross-device link',
  EBUSY: 'Device or resource busy',
  EDQUOT: 'Disk quota exceeded',
  EFBIG: 'File too large',
}

export function strerror(error: unknown): string {
  const code = errorCode(error)
  if (code !== null && STRERROR[code]) return STRERROR[code]!
  return errorMessage(error)
}

/** GNU's `quotearg` in the C locale: single quotes, an embedded quote as `'\''`. */
export function quoteC(name: string): string {
  return `'${name.replace(/'/g, "'\\''")}'`
}

export function tryHelp(program: string): string {
  return `Try '${program} --help' for more information.\n`
}
