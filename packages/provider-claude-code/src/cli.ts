export function buildClaudeArgs(params: {
  modelId: string
  systemPrompt: string
  thinkingEffort?: string | null
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
  return args
}

export function buildClaudeEnv(
  base: NodeJS.ProcessEnv = process.env,
  options: { oauthAccessToken?: string | null } = {},
): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {
    ...base,
    DISABLE_AUTO_COMPACT: '1',
    MAX_MCP_OUTPUT_TOKENS: '1000000',
  }
  delete env.CLAUDECODE
  const token = options.oauthAccessToken?.trim()
  if (token) env.CLAUDE_CODE_OAUTH_TOKEN = token
  return env
}
