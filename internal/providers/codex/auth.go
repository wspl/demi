package codex

import (
	"context"
	"fmt"
	"net/http"
	"strings"
	"time"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

//go:generate go run ../../../tools/contractgen

const clientID = "app_EMoamEEZ73f0CkXaXp7hrann"

// The Codex family's secret document: the ChatGPT sign-in's tokens, the
// account they act for, and when they were last refreshed. Demi defines it;
// it is not the Codex CLI's `auth.json`.
// +demi:root
type secret struct {
	AccessToken  provider.Secret `json:"accessToken"`
	RefreshToken provider.Secret `json:"refreshToken"`
	IDToken      provider.Secret `json:"idToken"`
	AccountID    accountID       `json:"accountId"`
	LastRefresh  core.Timestamp  `json:"lastRefresh"`
}

// A ChatGPT account id: nonempty text that a header can carry, since every
// request names the account in `ChatGPT-Account-ID`.
// +demi:root
// +demi:check validateAccountID
type accountID string

func validateAccountID(id accountID) error {
	if id == "" || !headerText(string(id)) {
		return fmt.Errorf("an account id is nonempty header text")
	}
	return nil
}

// headerText checks Codex's account and session header representation.
func headerText(s string) bool {
	for i := 0; i < len(s); i++ {
		if s[i] < 32 && s[i] != '\t' || s[i] == 127 {
			return false
		}
	}
	return true
}

type claims struct {
	Exp     provider.Reported[float64]       `json:"exp" wire:"optional"`
	Email   provider.ReportedString          `json:"email" wire:"optional"`
	Auth    provider.Reported[authClaims]    `json:"https://api.openai.com/auth" wire:"optional"`
	Profile provider.Reported[profileClaims] `json:"https://api.openai.com/profile" wire:"optional"`
}
type authClaims struct {
	AccountID provider.ReportedString `json:"chatgpt_account_id" wire:"optional"`
	Fedramp   provider.Reported[bool] `json:"chatgpt_account_is_fedramp" wire:"optional"`
}
type profileClaims struct {
	Email provider.ReportedString `json:"email" wire:"optional"`
}

func tokenClaims(token provider.Secret) claims {
	c := provider.JWTClaims(token.Expose(), vendorDecode[claims])
	if c == nil {
		return claims{}
	}
	return *c
}
func (c claims) email() *string {
	if c.Email.Value != nil {
		return c.Email.Value
	}
	if c.Profile.Value != nil {
		return c.Profile.Value.Email.Value
	}
	return nil
}
func (s secret) label() provider.AccountLabel {
	label := string(s.AccountID)
	email := tokenClaims(s.IDToken).email()
	if email == nil {
		email = tokenClaims(s.AccessToken).email()
	}
	if email != nil {
		label = *email
	}
	detail, identity := "chatgpt", string(s.AccountID)
	return provider.AccountLabel{Label: label, Detail: &detail, IdentityKey: &identity}
}
func (s secret) due(now core.Timestamp) bool {
	current, _ := now.Millisecond()        // Clock timestamps are validated by their owner.
	last, _ := s.LastRefresh.Millisecond() // Generated secret decoding validates timestamps.
	c := tokenClaims(s.AccessToken)
	if c.Exp.Value != nil {
		if expiry := provider.UnixSeconds(*c.Exp.Value); expiry != nil {
			ms, _ := expiry.Millisecond()
			if ms-current <= int64(5*time.Minute/time.Millisecond) {
				return true
			}
		}
	}
	return current-last >= int64(8*24*time.Hour/time.Millisecond)
}
func (p *Provider) document() (provider.AccountDocument, error) {
	if p.config.Account == nil {
		return nil, provider.AuthFailure{Family: "Codex", Reason: provider.AuthReasonMissing}
	}
	return p.pool.Document(*p.config.Account), nil
}
func (p *Provider) stored(ctx context.Context) (secret, error) {
	doc, err := p.document()
	if err != nil {
		return secret{}, err
	}
	stored, err := provider.ReadSecret(ctx, doc, decodeSecret)
	if err != nil {
		return secret{}, provider.AccountAuthFailure("Codex", err)
	}
	return stored.Secret, nil
}
func (p *Provider) credentials(ctx context.Context, client *http.Client, refused *provider.Secret) (secret, error) {
	doc, err := p.document()
	if err != nil {
		return secret{}, err
	}
	value, err := provider.Renew(ctx, doc, decodeSecret, func(s secret) bool { return refused != nil && s.AccessToken == *refused || s.due(p.clock.Now()) }, func(ctx context.Context, s secret) (secret, error) { return p.refresh(ctx, client, s) })
	if err != nil {
		return secret{}, provider.AccountAuthFailure("Codex", err)
	}
	return value, nil
}

type refreshedTokens struct {
	Access  provider.Secret  `json:"access_token"`
	Refresh *provider.Secret `json:"refresh_token"`
	ID      *provider.Secret `json:"id_token"`
}

//nolint:staticcheck // User-facing error text is copied verbatim from Rust.
func (p *Provider) refresh(ctx context.Context, client *http.Client, s secret) (secret, error) {
	response, err := postJSON(ctx, client, p.authEndpoint("/oauth/token"), map[string]string{"client_id": clientID, "grant_type": "refresh_token", "refresh_token": s.RefreshToken.Expose()})
	if err != nil {
		return secret{}, fmt.Errorf("Codex token refresh failed: %w", withoutURL(err))
	}
	defer func() { _ = response.Body.Close() }() // The reader reports IO failures; close releases the response.
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return secret{}, fmt.Errorf("Codex token refresh failed with HTTP %d", response.StatusCode)
	}
	tokens, err := provider.DecodeJSONResponse(ctx, response, vendorDecode[refreshedTokens])
	if err != nil {
		return secret{}, fmt.Errorf("Codex token refresh failed: %w", err)
	}
	s.AccessToken = tokens.Access
	if tokens.Refresh != nil {
		s.RefreshToken = *tokens.Refresh
	}
	if tokens.ID != nil {
		s.IDToken = *tokens.ID
	}
	s.LastRefresh = p.clock.Now()
	return s, nil
}

// vendorDecode adapts the shared vendor decoder to OAuth's secret-safe response reader.
func vendorDecode[T any](data []byte) (T, error) { return provider.DecodeUntagged[T](string(data)) }

func accountHeaders(s secret) http.Header {
	h := make(http.Header)
	h.Set("Authorization", s.AccessToken.Bearer().Expose())
	h.Set("Chatgpt-Account-Id", string(s.AccountID))
	for _, token := range []provider.Secret{s.IDToken, s.AccessToken} {
		c := tokenClaims(token)
		if c.Auth.Value != nil && c.Auth.Value.Fedramp.Value != nil && *c.Auth.Value.Fedramp.Value {
			h.Set("X-Openai-Fedramp", "true")
		}
	}
	h.Set("User-Agent", userAgent)
	return h
}
func inferenceHeaders(s secret, request provider.InferenceRequest) http.Header {
	h := accountHeaders(s)
	h.Set("Openai-Beta", "responses=experimental")
	h.Set("Accept", "text/event-stream")
	h.Set("Content-Type", "application/json")
	session := provider.PromptCacheKey(request.SessionID)
	if headerText(session) {
		h.Set("Session-Id", session)
		h.Set("Thread-Id", session)
	}
	if headerText(request.RequestID) {
		h.Set("X-Client-Request-Id", request.RequestID)
	}
	return h
}

// postJSON sends a vendor request using the shared wire encoder.
func postJSON(ctx context.Context, client *http.Client, address string, body any) (*http.Response, error) {
	data, err := provider.JSONBody(body)
	if err != nil {
		return nil, err
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodPost, address, strings.NewReader(string(data)))
	if err != nil {
		return nil, err
	}
	request.Header.Set("Content-Type", "application/json")
	return client.Do(request)
}
