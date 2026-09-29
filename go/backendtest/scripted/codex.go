package scripted

import (
	"encoding/base64"
	"encoding/json/v2"
	"sync"
	"testing"
	"time"
)

// Paths of the codex family's vendor, which Codex serves at its sign-in service
// and its ChatGPT backend; the scripted one serves both.
const (
	CodexUserCodePath = "/api/accounts/deviceauth/usercode"
	CodexPollPath     = "/api/accounts/deviceauth/token"
	CodexExchangePath = "/oauth/token"
	CodexUsagePath    = "/wham/usage"
)

// CodexUserCode and CodexAccountEmail are what the scripted sign-in reports: the
// code the user confirms, and the email of the account that signs in.
const (
	CodexUserCode     = "ABCD-1234"
	CodexAccountEmail = "device@example.test"
	// CodexRefreshToken is the refresh token the exchange hands out, a secret
	// that must never appear in the backend's files as it is.
	CodexRefreshToken = "codex-login-secret-refresh"
)

// A Codex is a scripted Codex vendor: the device login of its sign-in service,
// which reports a code and then waits until the scenario approves it, and the
// usage status of its backend. It is a Vendor, so it records every request.
type Codex struct {
	*Vendor

	mu       sync.Mutex
	approved bool
	percent  float64
}

// StartCodex starts a scripted Codex on a free loopback port.
func StartCodex(t testing.TB) *Codex {
	t.Helper()
	c := &Codex{Vendor: StartVendor(t), percent: 40}
	c.Handle(c.answer)
	return c
}

// Approve says whether the user confirmed the code: a poll before it is answered
// as one that is not yet confirmed.
func (c *Codex) Approve(approved bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.approved = approved
}

// Polls is how many times the login asked whether the code was confirmed.
func (c *Codex) Polls() int {
	count := 0
	for _, request := range c.Requests() {
		if request.Path == CodexPollPath {
			count++
		}
	}
	return count
}

// UseAt sets the used percent of the account's primary window.
func (c *Codex) UseAt(percent float64) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.percent = percent
}

func jsonAnswer(document any) *Response {
	encoded, err := json.Marshal(document, json.Deterministic(true))
	if err != nil {
		panic(err)
	}
	return Status(200).Header("content-type", "application/json").Chunk(encoded)
}

// tokenOf is a token that says what the backend reads of it: its expiry, the
// account's email and its ChatGPT account id. Its signature is not checked.
func tokenOf(claims map[string]any) string {
	encode := func(document any) string {
		encoded, err := json.Marshal(document, json.Deterministic(true))
		if err != nil {
			panic(err)
		}
		return base64.RawURLEncoding.EncodeToString(encoded)
	}
	return encode(map[string]any{"alg": "none"}) + "." + encode(claims) + ".signature"
}

func (c *Codex) answer(request Request) *Response {
	c.mu.Lock()
	defer c.mu.Unlock()
	switch request.Path {
	case CodexUserCodePath:
		// A short interval keeps the login's polls quick.
		return jsonAnswer(map[string]any{"device_auth_id": "device-auth-1", "user_code": CodexUserCode, "interval": 0.05})
	case CodexPollPath:
		if !c.approved {
			return Status(403)
		}
		return jsonAnswer(map[string]any{"authorization_code": "authorization-1", "code_verifier": "verifier-1"})
	case CodexExchangePath:
		claims := map[string]any{
			"exp":                         time.Now().Add(24 * time.Hour).Unix(),
			"email":                       CodexAccountEmail,
			"https://api.openai.com/auth": map[string]any{"chatgpt_account_id": "account-1"},
		}
		return jsonAnswer(map[string]any{
			"id_token": tokenOf(claims), "access_token": tokenOf(claims), "refresh_token": CodexRefreshToken,
		})
	case CodexUsagePath:
		return jsonAnswer(map[string]any{
			"plan_type": "plus",
			"rate_limit": map[string]any{"primary_window": map[string]any{
				"used_percent": c.percent, "limit_window_seconds": 7 * 24 * 3600, "reset_at": time.Now().Add(24 * time.Hour).Unix(),
			}},
		})
	}
	return nil
}
