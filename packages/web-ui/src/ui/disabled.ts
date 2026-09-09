/** Why a deferred product entry is disabled. */
export const IN_DEVELOPMENT = 'In development'

/** Tooltip text for a disabled control. Enabled or empty reasons yield nothing. */
export function disabledTooltip(
  disabled: boolean | undefined,
  reason: string | undefined,
): string | undefined {
  if (disabled !== true) {
    return undefined
  }
  const text = reason?.trim()
  return text || undefined
}
