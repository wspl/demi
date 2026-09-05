import type { ModelInfo, ModelServiceTier } from '../transport/protocol'

/** The model's Fast Mode tier, or null when the provider advertises none. */
export function fastServiceTier(model: ModelInfo | null | undefined): ModelServiceTier | null {
  return model?.serviceTiers?.find((tier) => tier.fast) ?? null
}

/** Fast Mode is on when the session's tier is the selected model's Fast tier. */
export function isFastMode(model: ModelInfo | null | undefined, serviceTierId: string | null | undefined): boolean {
  const fast = fastServiceTier(model)
  return fast != null && serviceTierId === fast.id
}
