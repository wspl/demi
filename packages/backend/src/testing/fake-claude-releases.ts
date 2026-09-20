const BASE = 'https://downloads.claude.ai/claude-code-releases'

/**
 * The Claude Code distribution for tests: one version whose only build is for
 * a platform no machine is, so an install is refused before any download. No
 * test reaches the vendor or fetches its executable.
 */
export function fakeClaudeReleases(version = '9.9.9'): { fetch: typeof fetch } {
  return {
    fetch: (async (input: string | URL | Request) => {
      const url = String(input)
      if (url === `${BASE}/latest`)
        return new Response(`${version}\n`)
      if (url === `${BASE}/${version}/manifest.json`)
        return Response.json({
          version,
          platforms: {
            'plan9-x64': { binary: 'claude', checksum: 'a'.repeat(64), size: 1 },
          },
        })
      return new Response('missing', { status: 404 })
    }) as typeof fetch,
  }
}
