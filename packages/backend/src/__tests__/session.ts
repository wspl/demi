import { createBackend, type Backend, type BackendOptions, type User } from '../index'
import { SESSION_COOKIE } from '../http/cookies'
import { modelsDevFetch } from './models-dev'

/** The master account every test backend is set up with. */
export const MASTER = { username: 'master', password: 'master-pass-1' }

/** A signed-in browser: its cookie on every request and stream socket. */
export interface WebSession {
  user: User
  cookie: string
  fetch(path: string, init?: RequestInit): Promise<Response>
  socket(path: string): WebSocket
}

export interface TestBackend extends Backend {
  /** The master, signed in by the setup call. */
  session: WebSession
}

/** A backend in shared mode unless said otherwise, the models.dev fixture behind its vendor catalog, with the master signed in: set up on a fresh data directory, logged in over a reopened one. */
export async function openBackend(options: Omit<BackendOptions, 'mode'> & { mode?: BackendOptions['mode'] }): Promise<TestBackend> {
  const backend = await createBackend({ mode: 'shared', modelsDev: { fetch: modelsDevFetch() }, ...options })
  try {
    const { needed } = (await (await fetch(`${backend.url}/api/setup`)).json()) as { needed: boolean }
    const session = needed ? await setupMaster(backend) : await login(backend, MASTER.username, MASTER.password)
    return { ...backend, session }
  } catch (error) {
    await backend.close()
    throw error
  }
}

export async function setupMaster(backend: Pick<Backend, 'url'>): Promise<WebSession> {
  return sessionFrom(backend.url, await postJson(backend.url, '/api/setup', MASTER), 201)
}

export async function login(backend: Pick<Backend, 'url'>, username: string, password: string): Promise<WebSession> {
  return sessionFrom(backend.url, await postJson(backend.url, '/api/auth/login', { username, password }), 200)
}

function postJson(url: string, path: string, body: unknown): Promise<Response> {
  return fetch(`${url}${path}`, { method: 'POST', body: JSON.stringify(body), headers: { 'content-type': 'application/json' } })
}

async function sessionFrom(url: string, response: Response, expectedStatus: number): Promise<WebSession> {
  if (response.status !== expectedStatus) throw new Error(`${response.url}: HTTP ${response.status} ${await response.text()}`)
  const token = response.headers.get('set-cookie')?.match(new RegExp(`${SESSION_COOKIE}=([^;]+)`))?.[1]
  if (!token) throw new Error('no session cookie in the response')
  const { user } = (await response.json()) as { user: User }
  return webSession(url, user, `${SESSION_COOKIE}=${token}`)
}

export function webSession(url: string, user: User, cookie: string): WebSession {
  return {
    user,
    cookie,
    fetch: (path, init) => fetch(`${url}${path}`, { ...init, headers: { ...(init?.headers as Record<string, string> | undefined), cookie } }),
    // Bun's WebSocket takes request headers; the browser sends the same-origin cookie by itself.
    socket: (path) => new WebSocket(`${url.replace(/^http/, 'ws')}${path}`, { headers: { cookie } }),
  }
}
