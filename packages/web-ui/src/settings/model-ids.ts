/**
 * The id a model added to a list takes: its own, or, when another model of
 * the list has it, the first free `-2`, `-3` and so on, which the user can
 * change (`crates/backend-user-shard/src/provider_settings.rs`).
 */
export function freeModelId(taken: readonly string[], id: string): string {
  if (!taken.includes(id)) {
    return id
  }
  let number = 2
  while (taken.includes(`${id}-${number}`)) {
    number += 1
  }
  return `${id}-${number}`
}
