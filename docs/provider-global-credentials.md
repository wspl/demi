# Provider global credentials (codex / claude-code / grok-build)

Contract and implementation notes for multi-credential support with a **global active** switch.

## Verified switching behavior

The current Codex, Claude Code, and Grok Build factories share an auth store
between login status, quota probing, and inference. `credentials.setActive(id)`
updates the stored selection and clears the quota instance owned by that provider.
Subsequent credential resolution reads the selected entry. Claude Code also
compares the active credential ID before each turn and replaces its retained CLI
process when the account changes.

An already started request can finish with its resolved credentials. Its quota
reply cannot restore the previous account's cache after a switch: old probes
reject with `ProviderQuotaInvalidatedError`, and captured inference observers
ignore invalidated replies. See [quota invalidation](provider-quota.md#5-relationship-to-credentials)
for the callback contract and the scope of the in-memory guarantee.

Fake credential fixtures verify account selection, auth resolution, and quota
invalidation in all three kits. Fake Claude transports verify that the next turn
uses a new process. Deferred probe and response tests verify late-result handling.
These tests do not call real models or establish that a vendor accepts a token.

Removing the active pool entry currently clears its pointer. A later pool-aware
lookup selects the first remaining entry by ID, or uses vendor defaults when the
pool is empty. This is the existing library behavior; product UI policy for
removing the current account remains separate.

## Pool read contract

`provider/src/credentials-pool.ts` owns the persisted metadata schema and derives
`CredentialEntryMeta` from it. IDs are non-empty directory-safe alphanumeric,
underscore or hyphen strings. Labels and present optional text fields contain a
non-whitespace character. `updatedAt` is an ISO datetime with an explicit offset.
Readers preserve valid values; they do not trim metadata, invent IDs, fill missing
timestamps, or replace malformed optional fields with null.

Only `ENOENT` yields an absent metadata/pointer result or an empty missing pool.
Malformed JSON, invalid fields, mismatched metadata IDs and invalid active pointers
raise `CredentialPoolError` with code `credential_invalid`, without including raw
file bodies. Other filesystem errors propagate. Listing fails on any incomplete or
invalid entry, including an entry removed concurrently during the listing; callers
may retry the complete operation. A missing pointer permits the documented first
entry selection. An existing pointer to missing metadata is invalid and does not
select a different account.

The active pointer format is one valid ID, optionally followed by a newline.
Removing an active entry clears the pointer. Clearing and setting the pointer must
report filesystem failures. A missing secret is `credential_not_found`; an unreadable
secret retains its IO error. Provider-specific secret schemas remain the concrete
provider's responsibility.

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

Authoritative multi-credential pool lives under the demi state root:

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
- A missing `active` pointer selects the first existing pool entry by ID. A malformed pointer or a pointer to missing metadata raises `CredentialPoolError`; it never selects a different identity. An empty pool follows the provider-specific default import behavior in §6.
- File mode `0600` for secret files; directories `0700` when created by demi.
- Credential ids: opaque stable strings (e.g. `import-<hash>` or product-supplied slug). Labels are display-only and may change.

**Not** under workspace cwd. Use `$DEMI_HOME` (default `~/.demi`); credential pool path helpers live in each provider kit — prefer provider-local layout constants next to each auth store.

## 6. Per-provider behavior

### 6.1 Common runtime pattern

Each of the three packages owns:

1. **Pool store** — list / read meta / read secret / write / setActive pointer.
2. **Active AuthStore** (or equivalent resolver) — `resolveAuth()` loads **only** the active entry’s material; refresh writes back into **that entry’s** secret file. Pool metadata timestamps record writes through `FileCredentialPool.writeEntry`; auth-file refresh does not rewrite metadata.
3. **`ProviderCredentials` adapter** wired on `create*Provider`.
4. **Import default** (optional helper, not automatic on every boot unless pool empty — see bootstrap).

Bootstrap (final-state default):

1. If the pool has entries, use `active`. A missing pointer selects the first entry by ID; an invalid pointer raises `CredentialPoolError` and does not change the selection.
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

`provider-codex/auth-schemas.ts` owns file, token refresh, device grant and consumed
claim schemas. `auth.ts` parses files before resolving their mode; `credentials.ts`
uses the same parser and pure `resolveCodexAuth` before writing an imported entry.
An incomplete credential cannot be imported as a generic `codex` entry. Missing
files report `auth_missing`; malformed files report `auth_invalid`; other import
IO failures retain their filesystem error. Unknown fields in valid vendor auth
files are preserved. `add` accepts exactly one of `authJsonText`, `authFile`,
`auth`, or `authJson`.

Absent or null optional API keys, token records, account IDs and refresh dates
represent missing material. Present token strings cannot be blank or contain
whitespace. Present refresh dates must be ISO datetimes. Token records validate
the fields they contain; mode resolution separately requires the credentials and
account ID needed by that mode. Unsupported Bedrock mode reports `auth_unsupported`.
JWT payloads supply metadata only; decoding does not verify a signature. Opaque
access tokens have no claims. JWT-shaped tokens must have a JSON object payload,
string account/email fields, boolean FedRAMP flags and a representable nonnegative
integer expiry when those fields exist.

Refresh responses require a new access token and validate optional ID/refresh
tokens. Claims are resolved before the atomic auth-file replacement, so a bad
refresh preserves the previous file. Locks and temporary files are released on
both success and failure; cleanup IO errors are reported.

Device login validates each response before polling or exchanging tokens. It
accepts `user_code` and the existing `usercode` variant, rejecting disagreement
when both appear. Poll intervals are integer seconds from 0 to 900, supplied as a
number or decimal digit string; an absent interval defaults to 5 seconds. Null,
blank strings, wrong types and out-of-range values fail validation. The 15-minute
poll window limits interval waits, and cancellation clears the pending timer.
Successful exchange requires ID, access and refresh tokens plus a resolvable
ChatGPT account ID. Protocol errors exclude response bodies.

### 6.3 Grok Build

`provider-grok-build/auth-schemas.ts` owns the vendor auth map, entry, OAuth token,
device authorization, user profile and consumed JWT claims. File reads and all
credential-add forms validate every entry before selection or writing. Each entry
requires a nonblank `key`; optional tokens, identity fields and ISO `expires_at`
must have their declared types when present. Null is not a substitute for missing
Grok fields. Unconsumed vendor extension fields remain intact. A missing file or
empty map is unauthenticated; malformed entries and IO failures are errors.

Pool secrets contain an auth map with one entry. The active metadata identity key
selects that entry. With an empty pool, the vendor map is selected by OIDC mode,
refresh-token availability and the auth.x.ai issuer, then lexical map-key order.
Import validates and copies all entries; default import activates that same
preferred entry. Explicit `{ entryKey, entry }`, `authJsonText` and `authFile` add
forms are mutually exclusive.

Refresh validates the raw response before replacing the selected entry, preserving
sibling entries and extension fields. `expires_in` is a finite nonnegative integer
number of seconds; strings and unrepresentable dates fail. A missing lifetime uses
the new token's expiration claim, or leaves expiry unknown. The old token's expiry
is not applied to its replacement. Missing refresh tokens retain the current
refresh token. The file lock protects reread, refresh and atomic replacement; a
fresh token written by another process is adopted. Failed validation preserves
the previous file. Temporary files and owned locks are released on failure, and
unexpected lock IO errors propagate.

Device authorization requires a positive lifetime and the base verification URI.
The optional complete URI uses the same checks: HTTPS or HTTP on localhost/
127.0.0.1, with no control characters. User codes contain only ASCII letters,
digits and hyphens. Missing polling interval defaults to five seconds; present
intervals are nonnegative integer seconds within the timer range. Polling waits at
least one second, adds five seconds for `slow_down`, and never extends the returned
expiration. Cancellation clears the wait and prevents the next request.

The optional `/user` enrichment may be unavailable due to a network failure or
non-success HTTP status. A successful response must satisfy its schema; malformed
profiles and cancellation are errors. Existing camel-case/snake-case profile
fields map explicitly to vendor entry fields. Team/organization token principals
seed account identity before optional profile enrichment.

Codex and Grok share `provider/validation.ts` JWT payload decoding. Opaque tokens
have no claims. JWT-shaped tokens require valid base64url, UTF-8 JSON and the
provider's consumed-claims schema. This extracts metadata and does not verify
signatures or confer authentication authority.

### 6.4 Claude Code

| Concern | Behavior |
|---|---|
| Active resolve | Pool entry `oauth.json` → access token |
| Inference | The transport resolves the configured auth store before spawning and injects file/static/env tokens through `CLAUDE_CODE_OAUTH_TOKEN`. Keychain tokens remain owned by the CLI and are not injected. |
| Quota probe | The provider factory injects its shared auth store into `createClaudeCodeQuota`. The standalone `resolveClaudeCodeOAuthAccess` helper reads env/keychain only. |
| Vendor default | If pool empty: env, then keychain |
| Import | Snapshot token from env or Keychain into pool |
| CLI constraint | Claude Code CLI still must accept token via env; if a future CLI ignores env, this path needs a different transport — out of scope until proven |

`provider-claude-code/auth-schemas.ts` owns the pool secret, access projection,
keychain envelope, OAuth response and credential-add schemas. `auth.ts` validates
the selected source before returning access. A configured OAuth file is
authoritative; an invalid file cannot fall back to another identity. Missing
files or absent default credentials are `auth_missing`. Malformed JSON and known
fields are `auth_invalid`; other file/keychain IO failures propagate. Optional
lookup helpers return null only for `auth_missing`. Quota and CLI consumers use
that same distinction, so corrupt auth stops a spawn instead of using CLI defaults.

Pool secrets require camel-case `accessToken`. Optional refresh tokens, expiry,
scopes and display metadata may be absent or null. Present tokens contain no
whitespace; expiry is an ISO datetime; scopes are arrays of nonempty strings
without whitespace. Wrong optional types are invalid. The vendor keychain has
its own schema: `claudeAiOauth.expiresAt` is an epoch in milliseconds. The platform
adapter treats the `security` command's item-not-found exit (44) as absent and
propagates other failures. This mapping follows Apple's
[command implementation](https://github.com/apple-oss-distributions/Security/blob/main/SecurityTool/macOS/keychain_find.c)
and [error constant](https://github.com/apple-oss-distributions/Security/blob/main/base/SecBase.h).
Tests inject both environment and keychain readers.

`login.ts` validates OAuth response tokens, numeric `expires_in`, space-separated
scope and account metadata before constructing a secret. Zero seconds means an
expired token; absent expiry means unknown (`null`). A refresh without new metadata
keeps existing metadata and the refresh token, but does not assign the previous
access token's expiry to the new token. Explicit force-refresh reaches the file
store through the pool-aware store. Renewed secrets are validated before atomic
replacement with mode 0600; temporary files are removed on success and failure.

Credential add accepts a secret directly or an `oauth` object and preserves its
refresh fields. Login, add and default import share the same entry writer and
identity rule: email when available, otherwise a token fingerprint. Default
import snapshots the resolved access token and display metadata. Login's pasted
input accepts a code or `code#state`, rejects extra fragments and verifies a
returned state. Cancelling the login stops waiting for pasted input.

## 7. Switch semantics

### 7.1 `setActive(id)`

1. Validate the entry metadata and require a readable secret file.
2. Write `active` pointer (atomic replace).
3. Invalidate quota cache for this provider.
4. Return `getActive()`; the concrete auth store parses the selected secret and reports its status.
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

`stateDir` defaults via `DEMI_HOME` / `~/.demi`; each provider kit resolves it with its own few lines.

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
