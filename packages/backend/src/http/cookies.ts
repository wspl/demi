import type { Context } from 'hono'
import { deleteCookie, getCookie, setCookie } from 'hono/cookie'

export const SESSION_COOKIE = 'demi_session'

export function readSessionCookie(c: Context): string | undefined {
  return getCookie(c, SESSION_COOKIE)
}

/** `HttpOnly; SameSite=Lax; Path=/`, `Secure` when the request came over https (directly or through a proxy). */
export function writeSessionCookie(c: Context, token: string, expiresAt: Date): void {
  setCookie(c, SESSION_COOKIE, token, { httpOnly: true, sameSite: 'Lax', path: '/', expires: expiresAt, secure: overHttps(c) })
}

export function clearSessionCookie(c: Context): void {
  deleteCookie(c, SESSION_COOKIE, { path: '/', secure: overHttps(c) })
}

function overHttps(c: Context): boolean {
  return c.req.url.startsWith('https:') || c.req.header('x-forwarded-proto')?.split(',')[0]?.trim() === 'https'
}
