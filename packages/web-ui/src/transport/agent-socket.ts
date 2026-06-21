import { AgentClient, createWebSocketClientTransport, type JsonWebSocket } from '@demi/agent'

export function agentSocketUrl(baseUrl: string, cwd: string): string {
  const url = new URL('/agent', baseUrl)
  url.protocol = url.protocol === 'https:' ? 'wss:' : url.protocol === 'http:' ? 'ws:' : url.protocol
  url.searchParams.set('cwd', cwd)
  return url.toString()
}

export function connectAgentClient(url: string): Promise<AgentClient> {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(url)
    socket.addEventListener(
      'open',
      () => resolve(new AgentClient(createWebSocketClientTransport(socket as unknown as JsonWebSocket))),
      { once: true },
    )
    socket.addEventListener('error', () => reject(new Error('Agent socket failed to connect')), { once: true })
  })
}
