---
'@demicodes/provider-grok-build': patch
---

Align Grok Build device login and cli-chat-proxy requests with the official CLI: frozen OAuth2 scopes including `api:access`, device-flow referrer/surface headers, proxy auth headers, and `/v1/billing?format=credits` (creditUsagePercent / currentPeriod).
