# Product contracts

Browser-safe schemas shared by the Demi backend, web product, and reusable UI.

- `auth` defines account input limits, normalization, and public identity.
- `preferences`, `models`, and `conversations` define product configuration.
- `responses` defines public REST and product state shapes.
- `frames` composes the agent protocol with upload and device file references.

Consumers import from `@demicodes/product-contracts`. The backend validates
requests at ingress; the browser validates responses at ingress. UI forms use
the same input schemas. Backend projections use the inferred response types.

Send and steer accept upload or device references and refuse stored attachment
records. Edit requests contain complete content, including retained attachment
records; unresolved upload or device references are rejected. The backend
resolves send and steer references before forwarding content to the agent.

See [data contracts](../../docs/data-contracts.md) and
[package boundaries](../../docs/package-boundaries.md) for ownership rules.
