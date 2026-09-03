import { resetModelsDevCacheForTests, type ModelsDevFetch } from '@demicodes/provider'

/**
 * A small models.dev catalog: two vendors our runtimes speak to (one
 * chat-completions, one anthropic), one we do not (bedrock), and GitHub
 * Copilot, which is excluded by name.
 */
export function modelsDevFixture(): unknown {
  return {
    deepseek: {
      id: 'deepseek',
      name: 'DeepSeek',
      npm: '@ai-sdk/openai-compatible',
      api: 'https://api.deepseek.com',
      doc: 'https://api-docs.deepseek.com',
      models: {
        'deepseek-v4': {
          name: 'DeepSeek V4',
          reasoning: true,
          tool_call: true,
          attachment: false,
          limit: { context: 128_000, output: 32_000 },
          cost: { input: 0.3, output: 1.2, cache_read: 0.03, cache_write: 0 },
        },
        'deepseek-v4-flash': { name: 'DeepSeek V4 Flash', limit: { context: 128_000 } },
      },
    },
    minimax: {
      id: 'minimax',
      name: 'MiniMax',
      npm: '@ai-sdk/anthropic',
      api: 'https://api.minimax.io/anthropic/v1',
      models: { 'minimax-m3': { name: 'MiniMax M3', limit: { context: 200_000, output: 64_000 } } },
    },
    'amazon-bedrock': {
      id: 'amazon-bedrock',
      name: 'Amazon Bedrock',
      npm: '@ai-sdk/amazon-bedrock',
      models: { 'anthropic.claude-sonnet': { name: 'Claude Sonnet on Bedrock' } },
    },
    'github-copilot': {
      id: 'github-copilot',
      name: 'GitHub Copilot',
      npm: '@ai-sdk/openai-compatible',
      api: 'https://api.githubcopilot.com',
      models: { 'gpt-5.5': { name: 'GPT-5.5' } },
    },
  }
}

/** A fetch serving the fixture; the shared cache is reset so each backend starts from it. */
export function modelsDevFetch(): ModelsDevFetch {
  resetModelsDevCacheForTests()
  return async () => Response.json(modelsDevFixture(), { headers: { etag: 'fixture' } })
}
