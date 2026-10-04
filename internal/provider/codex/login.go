package codex

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"strings"
	"time"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

type loginKit struct{ p *Provider }

// Capability reports the supported account operations.
func (*loginKit) Capability() provider.AccountsCapability {
	return provider.AccountsCapability{Login: true}
}

// Add reports that adding an account directly is unsupported.
func (*loginKit) Add(provider.AddAccount) (provider.NewAccount, error) {
	return provider.NewAccount{}, provider.ErrAccountsUnsupported
}

type userCodeAnswer struct {
	DeviceID  provider.NonEmpty     `json:"device_auth_id"`
	UserCode  *provider.NonEmpty    `json:"user_code"`
	Alternate *provider.NonEmpty    `json:"usercode"`
	Interval  provider.PollInterval `json:"interval"       wire:"optional"`
}
type authorization struct {
	Code     provider.Secret `json:"authorization_code"`
	Verifier provider.Secret `json:"code_verifier"`
}
type loginTokens struct {
	Access  provider.Secret `json:"access_token"`
	Refresh provider.Secret `json:"refresh_token"`
	ID      provider.Secret `json:"id_token"`
}

// Login completes device authorization and returns the new account.
func (k *loginKit) Login(ctx context.Context, pending func(types.LoginPending)) (provider.NewAccount, error) {
	return k.login(ctx, pending)
}

//nolint:staticcheck // ST1005: user-facing error text is a sentence starting with a capital letter.
func (k *loginKit) login(ctx context.Context, pending func(types.LoginPending)) (provider.NewAccount, error) {
	started := time.Now()
	response, err := k.post(ctx, "/api/accounts/deviceauth/usercode", map[string]string{"client_id": clientID})
	if err != nil {
		return provider.NewAccount{}, err
	}
	defer func() { _ = response.Body.Close() }() // The reader reports IO failures; close releases the response.
	if response.StatusCode == 404 {
		return provider.NewAccount{}, errors.New("Device-code login is not enabled for this Codex account")
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return provider.NewAccount{}, fmt.Errorf("Device code request failed with HTTP %d", response.StatusCode)
	}
	code, err := provider.DecodeJSONResponse(ctx, response, vendorDecode[userCodeAnswer])
	if err != nil {
		return provider.NewAccount{}, fmt.Errorf("Device code failed: %w", err)
	}
	userCode := code.UserCode
	if userCode == nil {
		userCode = code.Alternate
	}
	if userCode == nil {
		return provider.NewAccount{}, fmt.Errorf("Device code response is malformed: user_code is missing")
	}
	user := k.announceDeviceCode(userCode, pending)
	auth, err := k.authorize(ctx, code.DeviceID, user, code.Interval.Duration(), started)
	if err != nil {
		return provider.NewAccount{}, err
	}
	request, err := k.tokenRequest(ctx, auth)
	if err != nil {
		return provider.NewAccount{}, err
	}
	response, err = k.p.http.Do(request)
	if err != nil {
		return provider.NewAccount{}, fmt.Errorf("Codex sign-in request failed: %w", provider.WithoutURL(err))
	}
	defer func() { _ = response.Body.Close() }() // The reader reports IO failures; close releases the response.
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return provider.NewAccount{}, fmt.Errorf("Device-code token exchange failed with HTTP %d", response.StatusCode)
	}
	tokens, err := provider.DecodeJSONResponse(ctx, response, vendorDecode[loginTokens])
	if err != nil {
		return provider.NewAccount{}, fmt.Errorf("Token exchange failed: %w", err)
	}
	id, ok := loginAccountID(tokens)
	if !ok || validateAccountID(accountID(id)) != nil {
		return provider.NewAccount{}, fmt.Errorf("The Codex sign-in names no ChatGPT account")
	}
	return k.account(tokens, accountID(id))
}

//nolint:staticcheck // ST1005: user-facing error text is a sentence starting with a capital letter.
func (k *loginKit) authorize(
	ctx context.Context,
	device provider.NonEmpty,
	user string,
	interval time.Duration,
	started time.Time,
) (authorization, error) {
	for {
		response, err := k.post(
			ctx,
			"/api/accounts/deviceauth/token",
			map[string]string{"device_auth_id": string(device), "user_code": user},
		)
		if err != nil {
			return authorization{}, err
		}
		if response.StatusCode >= 200 && response.StatusCode < 300 {
			result, err := provider.DecodeJSONResponse(ctx, response, vendorDecode[authorization])
			if err != nil {
				return authorization{}, fmt.Errorf("Device authorization failed: %w", err)
			}
			return result, nil
		}
		_ = response.Body.Close() // Only the status is used; closing releases the rejected response.
		if response.StatusCode != 403 && response.StatusCode != 404 {
			return authorization{}, fmt.Errorf("Device authorization failed with HTTP %d", response.StatusCode)
		}
		if time.Since(started) >= provider.DeviceLoginLifetime {
			return authorization{}, fmt.Errorf("Device-code login timed out after 10 minutes")
		}
		timer := time.NewTimer(interval)
		select {
		case <-ctx.Done():
			timer.Stop()
			return authorization{}, ctx.Err()
		case <-timer.C:
		}
	}
}

//nolint:staticcheck // ST1005: user-facing error text starts with the product name Codex.
func (k *loginKit) post(ctx context.Context, path string, body any) (*http.Response, error) {
	response, err := postJSON(ctx, k.p.http, k.p.authEndpoint(path), body)
	if err != nil {
		return nil, fmt.Errorf("Codex sign-in request failed: %w", provider.WithoutURL(err))
	}
	return response, nil
}

func (k *loginKit) account(tokens loginTokens, id accountID) (provider.NewAccount, error) {
	s := secret{
		AccessToken:  tokens.Access,
		RefreshToken: tokens.Refresh,
		IDToken:      tokens.ID,
		AccountID:    id,
		LastRefresh:  k.p.clock.Now(),
	}
	data, err := provider.JSONBody(s)
	if err != nil {
		return provider.NewAccount{}, err
	}
	return provider.NewAccount{Secret: string(data), Label: s.label()}, nil
}

func (k *loginKit) tokenRequest(ctx context.Context, auth authorization) (*http.Request, error) {
	form := url.Values{
		"grant_type":    {"authorization_code"},
		"code":          {auth.Code.Expose()},
		"redirect_uri":  {k.p.authEndpoint("/deviceauth/callback")},
		"client_id":     {clientID},
		"code_verifier": {auth.Verifier.Expose()},
	}
	request, err := http.NewRequestWithContext(
		ctx,
		http.MethodPost,
		k.p.authEndpoint("/oauth/token"),
		strings.NewReader(form.Encode()),
	)
	if err != nil {
		return nil, err
	}
	request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	return request, nil
}

func loginAccountID(tokens loginTokens) (string, bool) {
	for _, token := range []provider.Secret{tokens.Access, tokens.ID} {
		c := tokenClaims(token)
		if c.Auth.Value != nil && c.Auth.Value.AccountID.Value != nil {
			return *c.Auth.Value.AccountID.Value, true
		}
	}
	return "", false
}

func (k *loginKit) announceDeviceCode(userCode *provider.NonEmpty, pending func(types.LoginPending)) string {
	ms, _ := k.p.clock.Now().Millisecond()
	expires := provider.UnixSeconds(float64(ms)/1000 + provider.DeviceLoginLifetime.Seconds())
	user := string(*userCode)
	pending(types.LoginPending{VerificationURL: k.p.authEndpoint("/codex/device"), UserCode: &user, ExpiresAt: expires})
	return user
}
