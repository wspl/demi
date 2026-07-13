---
'@demicodes/provider': minor
'@demicodes/provider-codex': minor
'@demicodes/provider-grok-build': minor
'@demicodes/provider-claude-code': minor
---

Native, remote-friendly credential logins for all pool providers.

`beginLogin` no longer spawns a vendor CLI. Each provider drives its public
login protocol directly and imports the result straight into the credential
pool, returning the `credentialId`:

- **codex** and **grok-build** run their public device-code grants; the
  verification URL and one-time code stream out via `onPending` so the user
  completes login from any browser on any device.
- **claude-code** runs the copy-back PKCE OAuth flow: `onPending` carries the
  authorize URL (`requiresCodeInput: true`), `promptForCode` collects the
  pasted `code#state` string, and pool secrets carry refresh tokens that renew
  on expiry.

New `ProviderCredentialLoginOptions` fields (`onPending`, `promptForCode`),
a `ProviderCredentialLoginPending` shape, and a `credentialId` on the completed
login result. `runVendorLoginCommand` is removed.
