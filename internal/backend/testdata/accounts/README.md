# Account scenario fixtures

The `claude_code/` distribution CA, certificate and key are fixed test
credentials. They authenticate only the
loopback scripted distribution in the opt-in Claude Code scenarios.

Run those scenarios with `-tags acceptance`, `DEMI_TEST_CLAUDE_CODE` naming the
real CLI, and `SSL_CERT_FILE` naming the absolute path of
`claude_code/distribution-ca.pem`. All model responses come from a local scripted
vendor; no real model is called.
