# Web authentication integration

The product's authentication is connected to the backend. The shared
`web-ui/auth/EmailLoginPage.vue` owns the form and its busy/error states;
`web/auth/LoginPage.vue` supplies the request handler. The gallery uses the same
component with fixture phases and never calls the backend.

`web/auth/session.ts` holds one session state: checking, signed out, or signed in
with a validated account. Its Zod schema validates account responses before they
enter the store. Passwords stay in the form/request and are cleared after login;
the browser receives no session token in JavaScript. The backend's HttpOnly
cookie accompanies same-origin requests.

Startup checks `GET /api/auth/me` before the initial route mounts. Further
navigation checks an existing session again. Missing authentication redirects
to `/login`; expiry during navigation clears the document and shows the session
ended message. A network/server failure during navigation reports the failure
and preserves the existing identity. Account state polling and API failures also detect expired sessions; all account-scoped stores and transports are released.

`POST /api/auth/login` supplies email and password. Backend errors, including
rate limiting, appear on the shared form. Unmounting the form aborts its pending
request, and a late response cannot change the session store. Requests time out
after 60 seconds. `POST /api/auth/logout` must succeed (or report an already-ended
session) before the document reload releases account-scoped stores and drafts.
Logout failure leaves the account signed in and shows an error.

## Development

Run the backend on port 3271 and `bun run web:dev` on port 18934. Vite forwards
`/api` HTTP and WebSocket requests to the backend. Set `DEMI_BACKEND_URL` when
the backend runs at another address. Initialize an account through the existing
backend setup API; this checkpoint adds no registration or administration UI.
Production serves the built web directory through `DEMI_WEB_DIRECTORY`.

## Account settings and product state

Nickname, verified email changes and password changes use the real account APIs.
Closing their dialogs aborts pending requests and clears temporary credentials.
The signed-in route opens the backend-integrated product; state polling, provider
configuration, devices, files and live conversations are described in
[web integration](web-integration.md). Tests use disposable accounts and fake mail
delivery, without real model calls.
