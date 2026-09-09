/** A missing nickname uses the email for display without changing account data. */
export function accountDisplayName(name: string, email?: string): string {
  return name.trim() || email?.trim() || 'Account'
}

export function accountInitial(name: string, email?: string): string {
  return Array.from(accountDisplayName(name, email))[0]!.toUpperCase()
}
