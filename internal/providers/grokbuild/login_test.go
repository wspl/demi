package grokbuild

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"reflect"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func device(t *testing.T, fields map[string]any) providertest.MockResponse {
	t.Helper()
	data := map[string]any{
		"device_code":      "dev_code_1",
		"user_code":        "GROK-1234",
		"verification_uri": "https://auth.x.ai/activate",
		"interval":         0,
		"expires_in":       600,
	}
	for k, v := range fields {
		data[k] = v
	}
	encoded, err := provider.JSONBody(data)
	if err != nil {
		t.Fatal(err)
	}
	return answer(200, string(encoded))
}

func confirmed(t *testing.T, access string, id *string) providertest.MockResponse {
	t.Helper()
	data := map[string]any{"access_token": access, "refresh_token": "rt_1", "expires_in": 3600}
	if id != nil {
		data["id_token"] = *id
	}
	encoded, err := provider.JSONBody(data)
	if err != nil {
		t.Fatal(err)
	}
	return answer(200, string(encoded))
}

func loginStored(t *testing.T, pool *provider.MemoryCredentialPool, id string) secret {
	t.Helper()
	entry, err := pool.Document(id).Read(context.Background())
	if err != nil || entry == nil {
		t.Fatalf("missing account: %v", err)
	}
	s, err := decodeSecret([]byte(entry.Text))
	if err != nil {
		t.Fatal(err)
	}
	return s
}

func TestDeviceLoginCLIContract(t *testing.T) {
	v := providertest.StartVendor(t)
	client := loginClient(t, v)
	v.RespondAt(
		"/oauth2/device/code",
		device(t, map[string]any{"verification_uri_complete": "https://auth.x.ai/activate?user_code=GROK-1234"}),
	)
	v.RespondAt("/oauth2/token", answer(400, `{"error":"authorization_pending"}`))
	id := providertest.JWT(t, map[string]any{"sub": "user_1", "email": "id@example.com"})
	v.RespondAt("/oauth2/token", confirmed(t, "at_1", &id))
	v.RespondAt("/v1/user", answer(200, `{"userId":"user_1","firstName":"G","email":"g@example.com"}`))
	pool := provider.NewMemoryCredentialPool()
	p := testProvider(v, pool, nil, client)
	synctest.Test(t, func(t *testing.T) {
		equal(t, provider.AccountsCapability{Login: true}, p.Accounts().Capability())
		var shown []core.LoginPending
		account, err := p.Accounts().Login(context.Background(), func(p core.LoginPending) { shown = append(shown, p) })
		if err != nil {
			t.Fatal(err)
		}
		equal(t, 1, len(shown))
		equal(t, "https://auth.x.ai/activate?user_code=GROK-1234", shown[0].VerificationURL)
		equal(t, "GROK-1234", *shown[0].UserCode)
		equal(t, core.Timestamp("2026-09-18T14:10:00.000Z"), *shown[0].ExpiresAt)
		equal(t, "g@example.com", account.Label)
		active, err := pool.Active(context.Background())
		if err != nil {
			t.Fatal(err)
		}
		equal(t, account.ID, *active)
		entry, err := pool.Document(account.ID).Read(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		equal(
			t,
			jsonValue(
				t,
				fmt.Sprintf(
					`{"accessToken":"at_1","refreshToken":"rt_1",`+
						`"expiresAt":"2026-09-18T15:00:00.000Z","issuer":%q,`+
						`"clientId":"b1a00492-073a-47ea-816f-4c329264a828",`+
						`"userId":"user_1","email":"g@example.com"}`,
					v.URL("/"),
				),
			),
			jsonValue(t, entry.Text),
		)
		listed, err := pool.List(context.Background())
		if err != nil {
			t.Fatal(err)
		}
		equal(t, v.URL("")+"::user_1", *listed[0].IdentityKey)
		requests := v.Requests()
		equal(t, 4, len(requests))
		for i, path := range []string{"/oauth2/device/code", "/oauth2/token", "/oauth2/token", "/v1/user"} {
			equal(t, path, requests[i].URI)
		}

		form, err := url.ParseQuery(string(requests[0].Body))
		if err != nil {
			t.Fatal(err)
		}
		equal(t, url.Values{"client_id": {clientID}, "scope": {scope}, "referrer": {"grok-build"}}, form)
		form, err = url.ParseQuery(string(requests[1].Body))
		if err != nil {
			t.Fatal(err)
		}
		equal(
			t,
			url.Values{
				"grant_type":  {"urn:ietf:params:oauth:grant-type:device_code"},
				"device_code": {"dev_code_1"},
				"client_id":   {clientID},
			},
			form,
		)
		for _, r := range requests[:3] {
			equal(t, "ui", r.Header("x-grok-client-surface"))
			equal(t, "1.0.5", r.Header("x-grok-client-version"))
		}
		equal(t, "Bearer at_1", requests[3].Header("authorization"))
		equal(t, "xai-grok-cli", requests[3].Header("x-xai-token-auth"))
		equal(t, "interactive", requests[3].Header("x-grok-client-mode"))
	})
	requests := v.Requests()
	// Rust wrote the fields in insertion order; Go's url.Values.Encode sorts
	// them. Field order means nothing to an OAuth server (RFC 6749 § 4.1.3,
	// RFC 8628 § 3.4), so the tech lead accepted the order as a normalization:
	// the bodies must hold exactly Rust's fields, each once, with its values.
	t.Run("form fields", func(t *testing.T) {
		const id = "b1a00492-073a-47ea-816f-4c329264a828"
		const scopes = `openid profile email offline_access grok-cli:access api:access ` +
			`conversations:read conversations:write workspaces:read workspaces:write`
		forms := []url.Values{
			{"client_id": {id}, "scope": {scopes}, "referrer": {"grok-build"}},
			{
				"grant_type":  {"urn:ietf:params:oauth:grant-type:device_code"},
				"device_code": {"dev_code_1"},
				"client_id":   {id},
			},
		}
		for i, want := range forms {
			got, err := url.ParseQuery(string(requests[i].Body))
			if err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(got, want) {
				t.Errorf("form %d: got %v; want %v", i, got, want)
			}
		}
	})
}

func TestTeamLoginIdentity(t *testing.T) {
	v := providertest.StartVendor(t)
	client := loginClient(t, v)
	v.RespondAt("/oauth2/device/code", device(t, nil))
	access := providertest.JWT(
		t,
		map[string]any{"sub": "user-42", "principal_type": "Team", "principal_id": "team-123"},
	)
	id := providertest.JWT(t, map[string]any{"sub": "user-42", "email": "member@example.com"})
	v.RespondAt("/oauth2/token", confirmed(t, access, &id))
	v.RespondAt("/v1/user", answer(404, `{}`))
	pool := provider.NewMemoryCredentialPool()
	p := testProvider(v, pool, nil, client)
	synctest.Test(t, func(t *testing.T) {
		account, err := p.Accounts().Login(context.Background(), func(core.LoginPending) {})
		if err != nil {
			t.Fatal(err)
		}
		equal(t, "team-123", account.Label)
		entry, err := pool.Document(account.ID).Read(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		document := jsonValue(t, entry.Text).(map[string]any)
		equal(t, "team-123", document["userId"])
		equal(t, jsonValue(t, `{"kind":"Team","id":"team-123"}`), document["principal"])
		if _, present := document["email"]; present {
			t.Fatal("team document includes email")
		}
	})
}

func TestLoginSlowDown(t *testing.T) {
	v := providertest.StartVendor(t)
	client := loginClient(t, v)
	v.RespondAt("/oauth2/device/code", device(t, map[string]any{"interval": 2}))
	v.RespondAt("/oauth2/token", answer(400, `{"error":"slow_down"}`))
	v.RespondAt("/oauth2/token", answer(400, `{"error":"authorization_pending"}`))
	v.RespondAt("/oauth2/token", confirmed(t, "at_1", nil))
	v.RespondAt("/v1/user", answer(404, `{}`))
	p := testProvider(v, provider.NewMemoryCredentialPool(), nil, client)
	synctest.Test(t, func(t *testing.T) {
		started := time.Now()
		_, err := p.Accounts().Login(context.Background(), func(core.LoginPending) {})
		if err != nil {
			t.Fatal(err)
		}
		equal(t, 16*time.Second, time.Since(started))
	})
}

func TestLoginTenMinuteDeadline(t *testing.T) {
	v := providertest.StartVendor(t)
	client := loginClient(t, v)
	v.RespondAt("/oauth2/device/code", device(t, map[string]any{"interval": 60, "expires_in": 1800}))
	for range 9 {
		v.RespondAt("/oauth2/token", answer(400, `{"error":"authorization_pending"}`))
	}
	pool := provider.NewMemoryCredentialPool()
	p := testProvider(v, pool, nil, client)
	synctest.Test(t, func(t *testing.T) {
		started := time.Now()
		var shown []core.LoginPending
		_, err := p.Accounts().Login(context.Background(), func(p core.LoginPending) { shown = append(shown, p) })
		if err == nil {
			t.Fatal("unconfirmed login succeeded")
		}
		var failure *provider.LoginError
		if !errors.As(err, &failure) || failure.Unavailable {
			t.Fatalf("expected failed login: %v", err)
		}
		equal(t, "Grok device login timed out before the user confirmed", failure.Error())
		equal(t, 600*time.Second, time.Since(started))
		equal(t, 1, len(shown))
		equal(t, core.Timestamp("2026-09-18T14:10:00.000Z"), *shown[0].ExpiresAt)
		equal(t, 10, len(v.Requests()))
		equal(t, 0, len(pool.Entries()))
	})
}

func TestRefusedAndUnsafeLogin(t *testing.T) {
	v := providertest.StartVendor(t)
	client := loginClient(t, v)
	v.RespondAt("/oauth2/device/code", device(t, nil))
	v.RespondAt("/oauth2/token", answer(400, `{"error":"access_denied"}`))
	v.RespondAt(
		"/oauth2/device/code",
		device(t, map[string]any{"verification_uri": "http://auth.example.com/activate"}),
	)
	v.RespondAt("/oauth2/device/code", device(t, map[string]any{"verification_uri": nil}))
	pool := provider.NewMemoryCredentialPool()
	p := testProvider(v, pool, nil, client)
	synctest.Test(t, func(t *testing.T) {
		for i, want := range []string{
			"Grok device login failed: access_denied",
			`Grok device code failed: the response is malformed at ` +
				`verification_uri: a field is missing, unknown or of the wrong type`,
			"Grok device code failed: the response names no verification_uri",
		} {
			_, err := p.Accounts().Login(context.Background(), func(core.LoginPending) {
				if i != 0 {
					t.Fatal("unsafe login showed code")
				}
			})
			if err == nil {
				t.Fatal("login succeeded")
			}
			var failure *provider.LoginError
			if !errors.As(err, &failure) || failure.Unavailable {
				t.Fatalf("expected failed login: %v", err)
			}
			equal(t, want, failure.Error())
		}
		equal(t, 0, len(pool.Entries()))
	})
}

func TestCancelPendingLogin(t *testing.T) {
	v := providertest.StartVendor(t)
	client := loginClient(t, v)
	v.RespondAt("/oauth2/device/code", device(t, map[string]any{"interval": 5}))
	pool := provider.NewMemoryCredentialPool()
	p := testProvider(v, pool, nil, client)
	synctest.Test(t, func(t *testing.T) {
		ctx, cancel := context.WithCancel(context.Background())
		defer cancel()
		_, err := p.Accounts().Login(ctx, func(core.LoginPending) { cancel() })
		if err == nil || !errors.Is(err, context.Canceled) {
			t.Fatalf("unexpected result: %v", err)
		}
		time.Sleep(time.Minute)
		equal(t, 1, len(v.Requests()))
		equal(t, "/oauth2/device/code", v.Requests()[0].URI)
		equal(t, 0, len(pool.Entries()))
	})
}

func TestTeamUserDetailsCannotReplacePrincipal(t *testing.T) {
	v := providertest.StartVendor(t)
	client := loginClient(t, v)
	v.RespondAt("/oauth2/device/code", device(t, nil))
	access := providertest.JWT(t, map[string]any{"principalType": "Organization", "principalId": "org-123"})
	v.RespondAt("/oauth2/token", confirmed(t, access, nil))
	v.RespondAt(
		"/v1/user",
		answer(200, `{"userId":"member","email":"member@example.com",`+
			`"principalType":"User","principalId":"member"}`),
	)
	pool := provider.NewMemoryCredentialPool()
	p := testProvider(v, pool, nil, client)
	synctest.Test(t, func(t *testing.T) {
		account, err := p.Accounts().Login(context.Background(), func(core.LoginPending) {})
		if err != nil {
			t.Fatal(err)
		}
		equal(t, "org-123", account.Label)
		s := loginStored(t, pool, account.ID)
		equal(t, "org-123", *s.UserID)
		if s.Email != nil {
			t.Fatal("organization keeps member email")
		}
	})
}

func TestLoginLongPollIntervalDeadline(t *testing.T) {
	v := providertest.StartVendor(t)
	client := loginClient(t, v)
	v.RespondAt("/oauth2/device/code", device(t, map[string]any{"interval": 601}))
	pool := provider.NewMemoryCredentialPool()
	p := testProvider(v, pool, nil, client)
	synctest.Test(t, func(t *testing.T) {
		started := time.Now()
		_, err := p.Accounts().Login(context.Background(), func(core.LoginPending) {})
		if err == nil {
			t.Fatal("unconfirmed login succeeded")
		}
		var failure *provider.LoginError
		if !errors.As(err, &failure) || failure.Unavailable {
			t.Fatalf("expected failed login: %v", err)
		}
		equal(t, "Grok device login timed out before the user confirmed", failure.Error())
		equal(t, 600*time.Second, time.Since(started))
		equal(t, 1, len(v.Requests()))
		equal(t, "/oauth2/device/code", v.Requests()[0].URI)
		equal(t, 0, len(pool.Entries()))
	})
}

// enrichmentTransport spends two seconds of fake time fetching user details.
// It keeps the scripted vendor's actual IO outside the synctest bubble.
type enrichmentTransport struct{ base http.RoundTripper }

func (d enrichmentTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	if request.URL.Path == "/v1/user" {
		timer := time.NewTimer(2 * time.Second)
		defer timer.Stop()
		select {
		case <-request.Context().Done():
			return nil, request.Context().Err()
		case <-timer.C:
		}
	}
	return d.base.RoundTrip(request)
}

func TestLoginConfirmedBeforeDeadlineFinishesEnrichment(t *testing.T) {
	v := providertest.StartVendor(t)
	client := loginClient(t, v)
	client.Transport = enrichmentTransport{base: client.Transport}
	v.RespondAt("/oauth2/device/code", device(t, map[string]any{"interval": 599}))
	v.RespondAt("/oauth2/token", confirmed(t, "at_1", nil))
	v.RespondAt("/v1/user", answer(200, `{"userId":"user_1","email":"g@example.com"}`))
	pool := provider.NewMemoryCredentialPool()
	p := testProvider(v, pool, nil, client)
	synctest.Test(t, func(t *testing.T) {
		started := time.Now()
		account, err := p.Accounts().Login(context.Background(), func(core.LoginPending) {})
		if err != nil {
			t.Fatal(err)
		}
		equal(t, 601*time.Second, time.Since(started))
		equal(t, "g@example.com", account.Label)
		s := loginStored(t, pool, account.ID)
		equal(t, "user_1", *s.UserID)
		equal(t, "g@example.com", *s.Email)
		equal(t, 3, len(v.Requests()))
	})
}
