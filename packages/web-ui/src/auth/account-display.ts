/** A missing nickname uses the email for display without changing account data. */
export function accountDisplayName(name: string, email?: string): string {
  return name.trim() || email?.trim() || 'Account'
}

/**
 * The letters an avatar shows: the first letters of the nickname's first two
 * words ("Alex Morgan" → AM, "Priya" → P, "李雷" → 李), else the first letter
 * of the email's local part.
 */
export function accountInitials(name: string, email?: string): string {
  const words = name.trim().split(/\s+/).filter(word => word !== '')
  const sources = words.length > 0 ? words.slice(0, 2) : [email?.trim().split('@')[0] ?? '']
  const letters = sources.map(source => Array.from(source)[0] ?? '').join('')
  return (letters || '?').toUpperCase()
}
