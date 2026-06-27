import { inject, provide, type InjectionKey } from 'vue'

/** Presentation options for the embedded agent UI, supplied by the host app. */
export interface AgentUiOptions {
  /** Show the provider/status icon at the start of each conversation tab. */
  showTabIcon: boolean
}

const DEFAULT_UI_OPTIONS: AgentUiOptions = {
  showTabIcon: true,
}

const KEY: InjectionKey<AgentUiOptions> = Symbol('agent-ui-options')

export function provideAgentUiOptions(options: Partial<AgentUiOptions> | undefined): void {
  provide(KEY, { ...DEFAULT_UI_OPTIONS, ...options })
}

export function useAgentUiOptions(): AgentUiOptions {
  return inject(KEY, DEFAULT_UI_OPTIONS)
}
