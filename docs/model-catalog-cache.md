# Model catalog caching

`backend/llm/model-catalog-cache.ts` owns the product's model-directory cache.
`provider` adapters fetch and validate vendor data. The backend caches only
model metadata, never credentials, authentication state, quota, or a session's
execution availability.

## Backend and database

Each provider entry has one persisted catalog in `control.sqlite`, accessed
through `ControlService`. Its key includes the provider configuration and active
subscription account. Different users and provider entries never share a row.
The row contains validated model metadata and the time of the successful check.
The backend holds the same record in memory. Both tiers use a 15-minute TTL.
A restart loads the database record before making a network request.

A fresh record returns immediately. An expired record returns immediately with
`stale: true` and starts one shared refresh. Concurrent readers join that refresh;
there is at most one upstream request per entry. A successful refresh replaces
the database and memory record. A failure retains the last successful catalog
and its timestamp, reports a warning, and delays automatic retries for one
minute. A cold cache waits for one bounded request; failure is explicit.
`GET /api/models?refresh=true` waits for the shared refresh and bypasses TTL and
retry delay. It never clears a usable directory before fetching a replacement.

Provider configuration edits, account changes, and deletion invalidate the
entry. An invalidated or cancelled request cannot repopulate a retired record.
Backend shutdown cancels and drains refresh work before closing storage.
Remote catalog requests have a timeout and receive cancellation signals.
Manual model configuration is read directly and never depends on a vendor fetch.
The existing adapter caches remain useful to direct framework consumers; backend
refreshes explicitly revalidate them instead of stacking their TTLs.

## Frontend

`web/state/product.ts` owns one catalog for the signed-in account. New and
existing conversations reuse it. A one-minute frontend TTL prevents ordinary
state polling and navigation from requesting the directory repeatedly. Concurrent
readers share one promise. Provider/account changes and explicit refresh revalidate;
signing out clears the snapshot, timer, and pending request.

`GET /api/models` is independent of conversation id. It includes provider
metadata and provider health. The frontend derives execution availability from
the existing device/workspace/Cloud snapshot for the selected conversation;
backend send admission continues to enforce the real target's availability.
Model metadata is never copied into per-conversation caches.

History loading does not await model discovery. Once the transcript arrives,
`web-ui` displays it while the composer independently loads model choices.
Model discovery failure does not turn a readable transcript into a loading or
failed conversation. Both product and gallery use the shared session and
composer loading states.

## Verification

Use fake clocks, deferred catalog fetches, and isolated SQLite databases. Verify
fresh hits, restart hits, TTL expiry, simultaneous readers, forced refresh,
failed refresh retention and retry delay, account/configuration invalidation,
shutdown cancellation, and invalid data rejection. Browser-state tests verify
one shared catalog across navigation, logout isolation, TTL refresh, and readable
history while model discovery is pending or fails. Never call real models.
