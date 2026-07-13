# Provider global credentials (codex / claude-code / grok-build)

Final-state design for multi-credential support with a **global active** switch.

## 1. Problem

Three subscription providers reuse vendor CLI / desktop login material:

| Provider | Default material | Nature |
|---|---|---|
| `codex` | `~/.codex/auth.json` (or `$CODEX_HOME`) | Single-file session; refresh writes back |
| `grok-build` | `~/.grok/auth.json` (or `$GROK_HOME`) | Multi-entry file, but runtime auto-picks one |
| `claude-code` | `CLAUDE_CODE_OAUTH_TOKEN` or macOS Keychain item | Single active OAuth for CLI + quota |

**Reality:** each backend is effectively one **process-global active login**, not N isolated injectables by default.

**Rejected approach:** multi-`Provider` instances per account (`codex:a`, `codex:b`). Without per-instance injectable material, instances share the same fixed path and only multiply shells. Concurrent different accounts per session is also not how these vendors work.

**Chosen approach:** keep **one** `Provider` id per backend; expose a **credentials** surface that lists known credentials and **switches the global active**. Selection of backend stays `ProviderSelection.providerId`; selection of account is orthogonal and global.

## 2. Goals and non-goals

### Goals

- Unified, host-agnostic **credentials API** on the public `Provider` shell (optional, like `auth` / `quota`).
- Support **multiple stored credentials** per provider family and **one global active** at a time.
- `setActive` is the only switch; **no** auto-rotation, health-based failover, or product “active switching system”.
- After switch: `auth.status`, `quota` (probe/observe/latest), and subsequent inference use the new active material.
- Secrets never cross AgentClient / browser-visible frames.
- Zero-config default: if demi has no pool yet, behavior matches today (read vendor default path / env / keychain as the sole active credential).
- **Native login**: product can ask demi to run a login flow. For codex the flow is demi-native device-code login (public protocol: user confirms at `auth.openai.com/codex/device` from any browser on any device; pending URL + one-time code stream out via `onPending`, the completed material imports straight into the pool and returns its `credentialId`). grok-build uses the same native pattern over auth.x.ai's standard RFC 8628 device grant. claude-code uses a copy-back OAuth flow: `onPending` carries the authorize URL (`requiresCodeInput: true`), the product collects the pasted "code#state" string via `promptForCode`, and demi finishes the PKCE exchange; pool entries carry the refresh token and renew on expiry.
- **Import** material created outside demi into the pool (`importDefault` snapshots the vendor default path / keychain; `add` registers product-supplied material). Both mint a stable `credentialId`.

### Non-goals

- Multi-instance `Provider` ids for accounts.
- Per-session or per-turn credential override on `ProviderSelection` / agent frames.
- Demi-owned OAuth **registration** (client ids are the vendors' public clients). All three providers drive their public login protocols natively — headless and remote hosts cannot run a vendor browser login, so demi never spawns vendor CLIs for login.
- API-key providers (`openai`, `anthropic`) in this design (they already take explicit keys; out of scope unless a later product wants the same pool shape).
- Bidirectional continuous sync with vendor CLIs as a product feature (import is one-shot or explicit; optional export is not required).

## 3. Core decisions

| Decision | Choice |
|---|---|
| Switch granularity | **Process-global active** per provider id |
| How products switch accounts | Call `provider.credentials.setActive(id)` (or control RPC wrapping it) |
| How products switch backends | Existing `ProviderSelection` / `set_provider` |
| How a new account enters the system | `beginLogin` native flow (codex / grok-build: device code; claude-code: copy-back code via `promptForCode`) → pool entry + `credentialId` returned directly; `importDefault` / `add` for material created outside demi |
| Where non-active material lives | Demi-managed pool under `$DEMI_HOME` / `~/.demi` |
| What inference reads | Always the **active** material via existing AuthStore / env resolution path |
| Concurrent sessions | All sessions on that provider share the same active account |
| In-flight turns | Keep using credentials resolved at request start; new active applies on the **next** `resolveAuth` / CLI spawn / quota probe |

### 3.1 Credential lifecycle (product view)

```text
[unauthenticated or wrong account]
        │
        ▼
 credentials.beginLogin({ onPending, promptForCode? })
        │
        │  onPending → product relays verificationUrl (+ userCode) to the user,
        │  who confirms from any browser on any device;
        │  copy-back flows collect the pasted code via promptForCode
        ▼
 { status: 'completed', credentialId }   ← material imported into the pool
        │
        ▼
 credentials.setActive(id)    ← process-global switch (first login auto-activates)
        │
        ▼
 auth / quota / inference use active
```

Adding a **second** account: beginLogin again (user confirms with the other account) → second `credentialId` → setActive as needed. `importDefault` / `add` cover material created outside demi (vendor CLI logins, env tokens, keychain).

## 4. Public contract (`@demicodes/provider`)

### 4.1 Types

```ts
/** Public metadata only — never tokens, cookies, or raw auth files. */
export interface ProviderCredentialInfo {
  /** Stable id within this provider (not globally unique across providers). */
  id: string
  /** Human label: email, account id, or import tag. */
  label: string
  /** Optional secondary display (plan name, issuer, …). */
  detail?: string | null
  /** ISO-8601 when this entry was last imported or refreshed in the pool. */
  updatedAt?: string | null
}

export interface ProviderCredentialActive {
  credentialId: string | null
  /** Same shape as today's auth status, for the active credential. */
  status: ProviderAuthState
}

export interface ProviderCredentials {
  /** Whether this provider supports a multi-credential pool + global switch. */
  capability(): ProviderCredentialsCapability

  list(): Promise<ProviderCredentialInfo[]> | ProviderCredentialInfo[]

  getActive(): Promise<ProviderCredentialActive> | ProviderCredentialActive

  /**
   * Make `credentialId` the process-global active credential for this provider.
   * Subsequent auth / quota / inference use it.
   * Throws if id unknown or material unusable.
   */
  setActive(credentialId: string): Promise<ProviderCredentialActive> | ProviderCredentialActive

  /**
   * Run the vendor's public login protocol natively (device code or copy-back
   * OAuth), surfacing user-facing material via `onPending`. On completion the
   * material is imported into the pool and its `credentialId` is returned.
   */
  beginLogin?(options?: ProviderCredentialLoginOptions): Promise<ProviderCredentialLoginResult>

  /**
   * Snapshot current vendor-default material into the demi pool.
   * Assigns a stable `id` and returns public metadata (no secrets).
   * For material created outside demi (vendor CLI login, env, keychain).
   */
  importDefault?(): Promise<ProviderCredentialInfo>

  /**
   * Register material supplied by the product (path or already-read payload).
   * Shape is provider-specific via a narrow tagged input; never accept browser-posted raw tokens
   * on a public web control method without an explicit trusted product path.
   */
  add?(input: ProviderCredentialAddInput): Promise<ProviderCredentialInfo>

  /** Remove a pool entry. If it was active, active becomes null or another entry per implementation policy. */
  remove?(credentialId: string): Promise<void>
}

/** User-facing material issued mid-flow; the product relays it to the user. */
export interface ProviderCredentialLoginPending {
  verificationUrl: string
  /** One-time code the user enters at the verification URL (device-code flows). */
  userCode?: string | null
  expiresAt?: string | null
  /** True when the user must bring a code back (`promptForCode` flows). */
  requiresCodeInput?: boolean
}

export interface ProviderCredentialLoginOptions {
  /** Abort the login flow. */
  signal?: AbortSignal
  /** Fires once when the flow issues user-facing material. */
  onPending?: (pending: ProviderCredentialLoginPending) => void
  /** Collects the code the user copied back from the vendor page. */
  promptForCode?: () => Promise<string>
}

export type ProviderCredentialLoginResult =
  | { status: 'completed'; credentialId: string } // material imported into the pool
  | { status: 'cancelled' }
  | { status: 'unavailable'; message: string }
  | { status: 'failed'; message: string }

/** Provider-specific add payloads; widen per kit without putting secrets on the shared type surface. */
export type ProviderCredentialAddInput = {
  /** Implementation-defined; concrete packages document accepted variants. */
  [key: string]: unknown
}

export type ProviderCredentialsCapability =
  | { mode: 'none' }
  | {
      mode: 'supported'
      /** Can run the vendor's native login flow (`beginLogin`). */
      canBeginLogin?: boolean
      /** Can import from the vendor default location into the pool. */
      canImportDefault?: boolean
      /** Can register externally supplied material (`add`). */
      canAdd?: boolean
      /** Pool can hold more than one credential. */
      multi?: boolean
    }
```

**Id rules**

- `beginLogin` / `importDefault` / `add` all **mint** a stable id (e.g. content hash of account identity claims, or uuid if unknown).
- Re-importing or re-logging-in the same account should upsert the same id (match on account email / accountId / entryKey) rather than duplicate rows.

### 4.2 `Provider` shell

```ts
export interface Provider {
  id: string
  displayName: string
  auth?: ProviderAuth
  quota?: ProviderQuota
  /** Optional multi-credential pool + global active switch. */
  credentials?: ProviderCredentials
  state?(): ...
  listModels?(): ...
}
```

### 4.3 Relationship to `auth`

- `auth.status()` **always** reflects the **active** credential (or unauthenticated if none).
- `credentials.getActive().status` is the same information as `auth.status()` when a credentials surface exists; products may use either.
- Providers without a pool omit `credentials` (`capability` effectively `none`); `auth` may still exist.

### 4.4 Relationship to `quota`

- `quota.probe` / `observeResponse` / `latest` always bind to the **active** credential.
- On successful `setActive`, implementations **must** clear in-memory quota cache for that provider (`latest` becomes null / stale) so the next `ensureQuota` does not show the previous account.
- `ProviderQuotaSnapshot.accountLabel` should match the active credential label when known.

### 4.5 Relationship to agent protocol

- **No** change to `ProviderSelection`, `open`, or `set_provider`.
- Agent runtime does not know about credential ids.
- Switching accounts is a **control-plane** concern (REPL flag, web control method, embedding host API), not a transcript/frame concern.

## 5. Storage layout (demi pool)

Authoritative multi-credential pool lives under demi state root (same root as command-bridge layout):

```text
$DEMI_HOME|~/.demi/
  credentials/
    codex/
      active          # utf-8 credential id + trailing newline
      entries/
        <credentialId>/
          meta.json   # { id, label, detail?, updatedAt, source }
          auth.json   # Codex auth.json shape (secret)
    grok-build/
      active
      entries/
        <credentialId>/
          meta.json
          auth.json   # single-entry or full Grok auth map slice
    claude-code/
      active
      entries/
        <credentialId>/
          meta.json
          oauth.json  # { accessToken, refreshToken?, … } secret
```

Rules:

- `meta.json` is listable; secret files are never returned from public APIs.
- `active` missing or pointing at a deleted entry → treat as unauthenticated / fall back to vendor default import path once (see §6).
- File mode `0600` for secret files; directories `0700` when created by demi.
- Credential ids: opaque stable strings (e.g. `import-<hash>` or product-supplied slug). Labels are display-only and may change.

**Not** under workspace cwd. Use `resolveDemiHome()` / `$DEMI_HOME` (host-local already owns this concept for bridges; credential pool path helpers may live in each provider kit or a small shared path helper in `utils` only if truly generic — prefer provider-local layout constants next to each auth store to avoid a new package).

## 6. Per-provider behavior

### 6.1 Common runtime pattern

Each of the three packages owns:

1. **Pool store** — list / read meta / read secret / write / setActive pointer.
2. **Active AuthStore** (or equivalent resolver) — `resolveAuth()` loads **only** the active entry’s material; refresh writes back into **that entry’s** secret file (and updates `meta.updatedAt`).
3. **`ProviderCredentials` adapter** wired on `create*Provider`.
4. **Import default** (optional helper, not automatic on every boot unless pool empty — see bootstrap).

Bootstrap (final-state default):

1. If pool has entries → use `active` (or first entry if `active` invalid, and repair `active`).
2. If pool empty → **read-through** vendor default (today’s path). Do **not** silently invent pool entries unless the product calls `importDefault` / `importFromPath` / explicit add.
3. Product that wants multi-cred: import A, import B, `setActive`.

Optional public helpers per package (root or documented internal used by products):

- `importDefaultCodexCredential(pool)` — copy current `~/.codex/auth.json` into pool, set active if first.
- Same for grok / claude-code.

### 6.2 Codex

| Concern | Behavior |
|---|---|
| Active resolve | Read `credentials/codex/entries/<active>/auth.json` via `FileCodexAuthStore`-like logic with `authFile` override; or pool-backed store implementing `CodexAuthStore` |
| Refresh | Write back to **that entry’s** `auth.json`, not necessarily `~/.codex/auth.json` |
| Vendor default | If pool empty: existing `FileCodexAuthStore({ codexHome })` |
| Import | Snapshot `auth.json` (+ derive label from email / accountId) |
| Multi-entry native | N/A — vendor file is single session; pool holds N snapshots |

`createCodexProvider` always constructs one provider id `codex` (override still allowed). Credentials surface is attached when a pool is enabled (default **on** with demi home, or always on with lazy empty pool).

### 6.3 Grok Build

| Concern | Behavior |
|---|---|
| Active resolve | Pool entry secret is either a full `auth.json` map with one preferred key, or a single entry payload + `entryKey` |
| Refresh | Update that entry in the pool secret file |
| Vendor default | If pool empty: today’s `FileGrokAuthStore` + `selectAuthEntry` |
| Import | For each OIDC entry in `~/.grok/auth.json`, or import selected entry only — product chooses; recommended **import-all-entries** as separate pool credentials |
| Native multi-entry | Vendor file can feed the pool; **runtime no longer auto-picks for multi-cred mode** — active pointer wins |

When pool is empty, keep current auto-pick for backward compatibility.

### 6.4 Claude Code

| Concern | Behavior |
|---|---|
| Active resolve | Pool entry `oauth.json` → access token |
| Inference | CLI child env: set `CLAUDE_CODE_OAUTH_TOKEN` from active entry (override process env for that spawn). Extend `buildClaudeEnv` / transport factory to accept token or env overlay from the active resolver |
| Quota probe | `resolveClaudeCodeOAuthAccess` becomes pool-aware (active entry first; else env; else keychain) |
| Vendor default | If pool empty: today’s env / keychain |
| Import | Snapshot token from env or Keychain into pool |
| CLI constraint | Claude Code CLI still must accept token via env; if a future CLI ignores env, this path needs a different transport — out of scope until proven |

Claude is the weakest link today (no AuthStore abstraction). Final-state requires:

- `ClaudeCodeAuthStore` (or `resolveAccess(): Promise<ClaudeCodeOAuthAccess>`) injectable into provider + quota + transport.
- Transport **must not** only call bare `buildClaudeEnv()` without active token overlay when a store is configured.

## 7. Switch semantics

### 7.1 `setActive(id)`

1. Validate entry exists and secret file parses.
2. Write `active` pointer (atomic replace).
3. Invalidate quota cache for this provider.
4. Return `getActive()` (re-resolved status).
5. Do **not** abort in-flight provider runs.
6. Do **not** rewrite `ProviderSelection` or restart AgentSession.

### 7.2 Visibility

| Surface | After `setActive` |
|---|---|
| Next inference `resolveAuth` / CLI spawn | New credential |
| In-flight stream | Old credential until that run ends |
| `auth.status()` | New |
| `quota.latest` | Cleared; next probe/observe is new account |
| Long-lived Claude CLI process (session reuse) | **Must restart** on next `run` if active credential id changed since process start (compare stored `credentialId` on `ActiveClaudeRun`) |

### 7.3 Errors

- Unknown id → throw typed error (`credential_not_found`).
- Corrupt secret → `auth` status `error`; `setActive` fails.
- Empty pool + no vendor default → `unauthenticated`.

## 8. Product / control plane

### 8.1 Library consumers

```ts
const provider = providers.find(p => p.id === 'codex')
await provider.credentials?.setActive('work')
```

### 8.2 Web control server (when product wants it)

Add control methods (names illustrative):

- `listCredentials` `{ providerId }` → `ProviderCredentialInfo[]`
- `getActiveCredential` `{ providerId }` → `ProviderCredentialActive`
- `setActiveCredential` `{ providerId, credentialId }` → `ProviderCredentialActive`
- `beginCredentialLogin` `{ providerId }` → `ProviderCredentialLoginResult` (long-running; pending material streams to the product via `onPending`)
- `importDefaultCredential` `{ providerId }` → `ProviderCredentialInfo`

Wire to `provider.credentials`. **Never** return secret fields.

`beginCredentialLogin` works on headless and remote hosts: the user completes the vendor confirmation from any browser on any device.

Web UI is optional follow-on; protocol types live with existing control protocol in web-ui transport.

### 8.3 REPL / CLI

Optional flags or subcommands later (`--credential`, `demi auth use`); not required for library completeness.

## 9. Package boundaries

| Package | Owns |
|---|---|
| `@demicodes/provider` | `ProviderCredentials*` types; optional tiny helpers if any (none required beyond types); document on `Provider` |
| `@demicodes/provider-codex` | Codex pool layout, import, `CodexAuthStore` pool implementation, wire `credentials` on `createCodexProvider` |
| `@demicodes/provider-grok-build` | Same for Grok |
| `@demicodes/provider-claude-code` | Auth resolver abstraction, env overlay on CLI spawn, pool, wire `credentials` |
| `@demicodes/host-local` | Unchanged for assembly; may document that `$DEMI_HOME/credentials` is reserved. Does **not** own provider secrets |
| `@demicodes/agent` | **No** credential APIs |
| `@demicodes/web` / web-ui | Optional control RPC + UI; secrets stay server-side |

Secret boundary (existing rule, reaffirmed): raw tokens and auth file bodies stay inside provider creators / auth stores; not in frames, not in `ProviderCredentialInfo`.

Public roots: export credentials types from `@demicodes/provider`; export import helpers and status helpers from each concrete provider root only if products need them; keep pool file IO behind implementation files if possible, or export a single `create*CredentialsPool` factory deliberately.

## 10. Factory wiring (illustrative)

```ts
// createCodexProvider — conceptual
const pool = options.credentialsPool ?? openCodexCredentialsPool({ stateDir: options.stateDir })
const authStore = options.authStore ?? createPoolBackedCodexAuthStore(pool)
const quota = createCodexQuota({ authStore })
return defineProvider({
  id: 'codex',
  displayName: 'Codex',
  auth: { status: () => authStore.status() },
  quota,
  credentials: createCodexCredentialsApi(pool, authStore, quota),
  listModels: () => listCodexModels({ authStore }),
  createRuntime: () => new CodexProvider({ authStore, quota, ... }),
})
```

`stateDir` defaults via `DEMI_HOME` / `~/.demi` (same resolution story as host-local; duplicate a 5-line resolver in each package or share a non-host helper in `utils` only if we already have path helpers — avoid depending on `host-local` from providers).

## 11. Testing

| Area | Coverage |
|---|---|
| `@demicodes/provider` | Type/export surface only if helpers added |
| Codex / Grok / Claude unit | Pool list / setActive / active resolve; refresh writes to entry not vendor home; quota cache cleared on switch |
| Claude unit | Spawn env contains active token; process restart when active id changes mid-session |
| Import | Snapshot from fixture auth.json / oauth fixture |
| Boundary | No secret fields on public credential info; platform-entrypoints if new exports |
| E2E (optional) | Real multi-file pool switch then probe quota / one short inference — gated by env |

## 12. Migration / compatibility

- Existing single-login users: pool empty → identical to current File/env/keychain behavior.
- No change to `ProviderSelection` or transcripts.
- Existing `authStore` injection remains: if caller passes `authStore`, that store is authoritative; `credentials` may be omitted or limited to `mode: 'none'` unless the store itself implements multi-cred.

## 13. Implementation order (final-state slices, not MVP product stages)

Land as coherent commits; each slice leaves main green:

1. **Contract** — types on `@demicodes/provider` + docs (`package-boundaries`, this file, add-a-provider note).
2. **Codex pool + credentials API** — highest leverage; AuthStore already clean.
3. **Grok pool + credentials API** — multi-entry import maps cleanly.
4. **Claude auth abstraction + env overlay + pool + credentials API** — required for real multi-cred on CLI path.
5. **Web control methods** (if product wants remote switch) + optional UI.

## 14. Summary

| Question | Answer |
|---|---|
| New provider per account? | **No** |
| Switch mechanism? | **Global `credentials.setActive`** |
| Login inside demi? | **Yes** — native public protocols (device code / copy-back OAuth); no vendor CLI spawn |
| How does an id appear? | **`beginLogin` / `importDefault` / `add`** all return one |
| Where do extras live? | **`$DEMI_HOME/credentials/<providerId>/`** |
| What does inference use? | **Active only**, via AuthStore / env |
| Agent protocol change? | **None** |
| Auto switch? | **No** — interface only |
| Three providers? | **Yes**, same contract; Claude needs auth inject first |

This is the complete library/product-boundary design. Implementation follows §13.
