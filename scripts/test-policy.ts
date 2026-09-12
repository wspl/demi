/** Explicit external-service opt-ins are disabled by the normal regression command. */
export function externalTestEnabled(
  flag: string,
  env: Record<string, string | undefined> = process.env,
): boolean {
  return env.DEMI_TEST_MODE !== 'offline' && env[flag] === '1'
}
