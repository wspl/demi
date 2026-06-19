export function buildClaudeArgs(params: {
  modelId: string
  systemPrompt: string
  thinkingEffort?: string | null
  maxBudgetUsd?: number | string | null
}): string[] {
  const args = [
    '--print',
    '--output-format',
    'stream-json',
    '--verbose',
    '--input-format',
    'stream-json',
    '--include-partial-messages',
    '--no-session-persistence',
    '--safe-mode',
    '--disable-slash-commands',
    '--tools',
    '',
    '--permission-mode',
    'bypassPermissions',
    '--allow-dangerously-skip-permissions',
    '--model',
    params.modelId,
    '--system-prompt',
    params.systemPrompt,
  ]
  if (params.thinkingEffort) args.push('--effort', params.thinkingEffort)
  if (params.maxBudgetUsd !== undefined && params.maxBudgetUsd !== null) {
    args.push('--max-budget-usd', String(params.maxBudgetUsd))
  }
  return args
}

export function buildClaudeEnv(base: NodeJS.ProcessEnv = process.env): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {
    ...base,
    DISABLE_AUTO_COMPACT: '1',
    MAX_MCP_OUTPUT_TOKENS: '1000000',
  }
  delete env.CLAUDECODE
  return env
}
