package codex_test

import (
	"context"
	"errors"
	"net/http"
	"net/url"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/providers/codex"
)

func TestDeviceLogin(t *testing.T) {
	v := providertest.StartVendor(t)
	pool := provider.NewMemoryCredentialPool()
	p := configured(t, v, pool, func(c *codex.Config) { c.Account = nil })
	v.Respond(answer(200, `{"device_auth_id":"dev_auth_1","user_code":"WXYZ-9876","interval":"0"}`))
	v.Respond(answer(403, `{}`))
	v.Respond(answer(200, `{"authorization_code":"authz_1","code_challenge":"challenge",`+
		`"code_verifier":"verifier_1"}`))
	access := providertest.JWT(
		t,
		map[string]any{
			"email":                       "device@example.com",
			"https://api.openai.com/auth": map[string]any{"chatgpt_account_id": "acct-device"},
		},
	)
	data, err := provider.JSONBody(
		map[string]string{
			"access_token":  access,
			"refresh_token": "refresh_1",
			"id_token":      providertest.JWT(t, map[string]string{"email": "device@example.com"}),
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	v.Respond(answer(200, string(data)))
	equal(t, p.Accounts().Capability(), provider.AccountsCapability{Login: true})
	var shown []core.LoginPending
	info, err := p.Accounts().Login(t.Context(), func(pending core.LoginPending) { shown = append(shown, pending) })
	if err != nil {
		t.Fatal(err)
	}
	equal(t, len(shown), 1)
	equal(t, *shown[0].UserCode, "WXYZ-9876")
	equal(t, shown[0].VerificationURL, v.URL("/codex/device"))
	equal(t, *shown[0].ExpiresAt, core.Timestamp("2026-09-18T14:10:00.000Z"))
	equal(t, info.Label, "device@example.com")
	active, err := p.Accounts().Active(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	equal(t, *active, info.ID)
	stored, err := pool.Document(info.ID).Read(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	doc := jsonObject(t, []byte(stored.Text))
	equal(t, doc["accountId"], "acct-device")
	equal(t, doc["refreshToken"], "refresh_1")
	equal(t, doc["lastRefresh"], string(now))
	requests := v.Requests()
	equal(t, len(requests), 4)
	equal(
		t,
		[]string{requests[0].URI, requests[1].URI, requests[2].URI, requests[3].URI},
		[]string{
			"/api/accounts/deviceauth/usercode",
			"/api/accounts/deviceauth/token",
			"/api/accounts/deviceauth/token",
			"/oauth/token",
		},
	)
	equal(t, requests[0].JSON(t), map[string]any{"client_id": "app_EMoamEEZ73f0CkXaXp7hrann"})
	equal(t, requests[1].JSON(t), map[string]any{"device_auth_id": "dev_auth_1", "user_code": "WXYZ-9876"})
	form, err := url.ParseQuery(string(requests[3].Body))
	if err != nil {
		t.Fatal(err)
	}
	equal(
		t,
		form,
		url.Values{
			"grant_type":    {"authorization_code"},
			"code":          {"authz_1"},
			"code_verifier": {"verifier_1"},
			"client_id":     {"app_EMoamEEZ73f0CkXaXp7hrann"},
			"redirect_uri":  {v.URL("/deviceauth/callback")},
		},
	)
}

func TestDeviceCodeMissingAndUnavailable(t *testing.T) {
	v := providertest.StartVendor(t)
	pool := provider.NewMemoryCredentialPool()
	p := configured(t, v, pool, func(c *codex.Config) { c.Account = nil })
	v.Respond(answer(200, `{"device_auth_id":"dev_auth_1"}`))
	v.Respond(answer(404, `{}`))
	for _, test := range []struct {
		message     string
		unavailable bool
	}{{
		"Device code response is malformed: user_code is missing",
		false,
	}, {
		"Device-code login is not enabled for this Codex account",
		true,
	}} {
		_, err := p.Accounts().Login(t.Context(), func(core.LoginPending) { t.Error("unexpected code") })
		var login *provider.LoginError
		if !errors.As(err, &login) {
			t.Fatalf("error: %v", err)
		}
		equal(t, login.Error(), test.message)
		equal(t, login.Unavailable, test.unavailable)
	}
	equal(t, len(pool.Entries()), 0)
}

func TestDeviceLoginLifetime(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		calls := 0
		client := &http.Client{Transport: roundTripFunc(func(r *http.Request) (*http.Response, error) {
			calls++
			if strings.HasSuffix(r.URL.Path, "/usercode") {
				return controlledResponse(
					200,
					`{"device_auth_id":"dev_auth_1","user_code":"CODE-1","interval":60}`,
				), nil
			}
			return controlledResponse(403, `{}`), nil
		})}
		pool := provider.NewMemoryCredentialPool()
		p, err := codex.New(
			codex.NewConfig(nil),
			pool,
			&provider.MemorySnapshots{},
			client,
			providertest.FixedClock(now),
		)
		if err != nil {
			t.Fatal(err)
		}
		start := time.Now()
		_, err = p.Accounts().Login(t.Context(), func(core.LoginPending) {})
		var login *provider.LoginError
		if !errors.As(err, &login) {
			t.Fatalf("login error: %v", err)
		}
		equal(t, login.Unavailable, false)
		equal(t, login.Error(), "Device-code login timed out after 10 minutes")
		equal(t, time.Since(start), 10*time.Minute)
		equal(t, calls, 12)
		equal(t, len(pool.Entries()), 0)
	})
}

func TestCancelDeviceLogin(t *testing.T) {
	v := providertest.StartVendor(t)
	pool := provider.NewMemoryCredentialPool()
	p := configured(t, v, pool, func(c *codex.Config) { c.Account = nil })
	v.Respond(answer(200, `{"device_auth_id":"dev_auth_1","usercode":"CODE-1","interval":5}`))
	v.Respond(answer(403, `{}`))
	ctx, cancel := context.WithCancel(t.Context())
	done := make(chan struct{})
	var loginErr error
	go func() {
		defer close(done)
		_, loginErr = p.Accounts().Login(ctx, func(core.LoginPending) {})
	}()
	defer func() {
		cancel()
		<-done
	}()
	v.Received(t.Context(), 2)
	cancel()
	<-done
	if !errors.Is(loginErr, context.Canceled) {
		t.Fatalf("error: %v", loginErr)
	}
	equal(t, len(v.Requests()), 2)
	equal(t, len(pool.Entries()), 0)
}
