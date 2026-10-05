/** The operating system a web browser runs on. */
export type ClientPlatform = 'mac' | 'windows' | 'linux' | 'other'

/** The operating system of the browser whose `navigator` this is. */
export function clientPlatform(agent: { platform?: string; userAgent: string }): ClientPlatform {
  const name = `${agent.platform ?? ''} ${agent.userAgent}`
  if (/mac/i.test(name)) {
    return 'mac'
  }
  if (/win/i.test(name)) {
    return 'windows'
  }
  return /linux|android|cros/i.test(name) ? 'linux' : 'other'
}
