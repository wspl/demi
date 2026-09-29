package codex

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"time"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

type accountKit struct {
	http    *http.Client
	authURL url.URL
	clock   core.Clock
}

func (*accountKit) Capability() provider.AccountsCapability {
	return provider.AccountsCapability{Login: true}
}
func (*accountKit) Add(provider.AddAccount) (provider.NewAccount, error) {
	return provider.NewAccount{}, provider.ErrAddUnsupported
}
func (k *accountKit) endpoint(path string) string {
	endpoint, err := provider.EndpointURL(&k.authURL, path)
	if err != nil {
		panic(err)
	} // The configuration's parsed URL and constant path are valid.
	return endpoint.String()
}

//demi:wire
type deviceCodeRequest struct {
	ClientID string `json:"client_id"`
}

//demi:wire open
type deviceCodeAnswer struct {
	DeviceAuthID string                 `json:"device_auth_id" check:"chars=1.."`
	UserCode     *string                `json:"user_code,omitzero" check:"nullabsent,chars=1.."`
	Usercode     *string                `json:"usercode,omitzero" check:"nullabsent,chars=1.."`
	Interval     *provider.PollInterval `json:"interval,omitzero" check:"nullabsent,func=provider.Validate"`
}

//demi:wire
type authorizationRequest struct {
	DeviceAuthID string `json:"device_auth_id"`
	UserCode     string `json:"user_code"`
}

//demi:wire open
type authorizationAnswer struct {
	AuthorizationCode provider.Secret `json:"authorization_code" check:"func=provider.Validate"`
	CodeVerifier      provider.Secret `json:"code_verifier" check:"func=provider.Validate"`
}

//demi:wire open
type loginTokens struct {
	IDToken      provider.Secret `json:"id_token" check:"func=provider.Validate"`
	AccessToken  provider.Secret `json:"access_token" check:"func=provider.Validate"`
	RefreshToken provider.Secret `json:"refresh_token" check:"func=provider.Validate"`
}

// Login follows Codex's nonstandard authorization-code device protocol. Unlike
// RFC 8628, it polls JSON with vendor ids, treats 403/404 as pending, and uses
// a server-issued PKCE verifier in a separate authorization-code exchange.
func (k *accountKit) Login(ctx context.Context, pending func(core.LoginPending)) (provider.NewAccount, error) {
	started := time.Now()
	response, err := post(ctx, k.http, k.endpoint("/api/accounts/deviceauth/usercode"), "application/json", provider.JSONBody(deviceCodeRequest{ClientID: ClientID}))
	if err != nil {
		return provider.NewAccount{}, err
	}
	if response.StatusCode == 404 {
		response.Body.Close()
		return provider.NewAccount{}, fmt.Errorf("Device-code login is not enabled for this Codex account")
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		response.Body.Close()
		return provider.NewAccount{}, fmt.Errorf("Device code request failed with HTTP %d", response.StatusCode)
	}
	code, err := provider.DecodeOAuthResponse(response, decode[deviceCodeAnswer])
	if err != nil {
		return provider.NewAccount{}, fmt.Errorf("Device code failed: %w", err)
	}
	userCode := code.UserCode
	if userCode == nil {
		userCode = code.Usercode
	}
	if userCode == nil {
		return provider.NewAccount{}, fmt.Errorf("Device code response is malformed: user_code is missing")
	}
	expires, expiryErr := core.TimestampFromMillisecond(k.clock.Now().Millisecond() + 600000)
	var expiresAt *core.Timestamp
	if expiryErr == nil {
		expiresAt = &expires
	}
	pending(core.LoginPending{VerificationURL: k.endpoint("/codex/device"), UserCode: userCode, ExpiresAt: expiresAt})
	interval := 5.0
	if code.Interval != nil {
		interval = code.Interval.Seconds()
	}
	var authorization authorizationAnswer
	for {
		response, err := post(ctx, k.http, k.endpoint("/api/accounts/deviceauth/token"), "application/json", provider.JSONBody(authorizationRequest{DeviceAuthID: code.DeviceAuthID, UserCode: *userCode}))
		if err != nil {
			return provider.NewAccount{}, err
		}
		if response.StatusCode >= 200 && response.StatusCode < 300 {
			authorization, err = provider.DecodeOAuthResponse(response, decode[authorizationAnswer])
			if err != nil {
				return provider.NewAccount{}, fmt.Errorf("Device authorization failed: %w", err)
			}
			break
		}
		response.Body.Close()
		if response.StatusCode != 403 && response.StatusCode != 404 {
			return provider.NewAccount{}, fmt.Errorf("Device authorization failed with HTTP %d", response.StatusCode)
		}
		if time.Since(started) >= provider.DeviceLoginLifetime {
			return provider.NewAccount{}, fmt.Errorf("Device-code login timed out after 10 minutes")
		}
		// A very large vendor interval remains cancellable without overflowing Duration.
		seconds := min(interval, float64((1<<63-1)/int64(time.Second)))
		timer := time.NewTimer(time.Duration(seconds * float64(time.Second)))
		select {
		case <-ctx.Done():
			timer.Stop()
			return provider.NewAccount{}, ctx.Err()
		case <-timer.C:
		}
	}
	form := url.Values{"grant_type": {"authorization_code"}, "code": {authorization.AuthorizationCode.Expose()}, "redirect_uri": {k.endpoint("/deviceauth/callback")}, "client_id": {ClientID}, "code_verifier": {authorization.CodeVerifier.Expose()}}
	response, err = post(ctx, k.http, k.endpoint("/oauth/token"), "application/x-www-form-urlencoded", []byte(form.Encode()))
	if err != nil {
		return provider.NewAccount{}, err
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		response.Body.Close()
		return provider.NewAccount{}, fmt.Errorf("Device-code token exchange failed with HTTP %d", response.StatusCode)
	}
	tokens, err := provider.DecodeOAuthResponse(response, decode[loginTokens])
	if err != nil {
		return provider.NewAccount{}, fmt.Errorf("Token exchange failed: %w", err)
	}
	id := accountOf(tokens.AccessToken).accountID
	if id == nil {
		id = accountOf(tokens.IDToken).accountID
	}
	if id == nil || accountID(*id) != nil {
		return provider.NewAccount{}, fmt.Errorf("The Codex sign-in names no ChatGPT account")
	}
	secret := SecretDocument{AccessToken: tokens.AccessToken, RefreshToken: tokens.RefreshToken, IDToken: tokens.IDToken, AccountID: *id, LastRefresh: k.clock.Now()}
	encoded, err := secret.JSON()
	if err != nil {
		return provider.NewAccount{}, err
	}
	return provider.NewAccount{Secret: string(encoded), Label: secret.label()}, nil
}
