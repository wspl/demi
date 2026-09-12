/** Provider visibility and execution capability are independent of model metadata. */
export function providerCanRun(
  available: boolean,
  requiresProcessCapableHost: boolean,
  executionAvailable: boolean,
): boolean {
  return available && (!requiresProcessCapableHost || executionAvailable)
}
