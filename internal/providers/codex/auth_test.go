package codex_test

import (
	"context"
	"strings"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/providers/codex"
)

func TestRefusedRequestRefreshesOnce(t *testing.T) {
	v, pool, p := setup(t)
	v.RespondAt(responses, answer(401, `{"error":{"message":"expired"}}`))
	v.RespondAt("/oauth/token", answer(200, `{"access_token":"new-access","refresh_token":"refresh-2"}`))
	v.RespondAt(responses, completed())
	events := run(t.Context(), t, p, v.Client(), providertest.InferenceRequest())
	equal(t, events, []provider.Event{&provider.Response{Usage: core.TokenUsage{InputTokens: 1, OutputTokens: 1}}})
	requests := v.Requests()
	equal(t, len(requests), 3)
	equal(t, []string{requests[0].URI, requests[1].URI, requests[2].URI}, []string{responses, "/oauth/token", responses})
	equal(t, requests[0].Header("Authorization"), "Bearer "+freshToken(t))
	equal(t, requests[2].Header("Authorization"), "Bearer new-access")
	equal(t, requests[1].JSON(t), map[string]any{"client_id": "app_EMoamEEZ73f0CkXaXp7hrann", "grant_type": "refresh_token", "refresh_token": "refresh-1"})
	stored, err := pool.Document(account).Read(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	got := jsonObject(t, []byte(stored.Text))
	equal(t, got["accessToken"], "new-access")
	equal(t, got["refreshToken"], "refresh-2")
	equal(t, got["accountId"], "acct-1")
	equal(t, got["lastRefresh"], string(now))
}
func TestCompetingRefresherIsAdopted(t *testing.T) {
	v, pool, p := setup(t)
	v.RespondAt(responses, answer(401, ""))
	v.RespondAt(responses, completed())
	doc := pool.Document(account)
	permit, err := doc.RefreshTurn(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(t.Context())
	done := make(chan struct{})
	var events []provider.Event
	go func() {
		defer close(done)
		events = run(ctx, t, p, v.Client(), providertest.InferenceRequest())
	}()
	t.Cleanup(func() {
		cancel()
		permit.Release()
		<-done
	})
	v.Received(t.Context(), 1)
	stored, err := doc.Read(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	replacement := document(t, "rotated-access", "refresh-2", now)
	kept, err := doc.Replace(t.Context(), replacement, stored.Version)
	if err != nil || !kept {
		t.Fatalf("replace: %v, %v", kept, err)
	}
	permit.Release()
	<-done
	equal(t, len(events), 1)
	if _, ok := events[0].(*provider.Response); !ok {
		t.Fatalf("events: %v", events)
	}
	equal(t, len(v.Requests()), 2)
	equal(t, []string{v.Requests()[0].URI, v.Requests()[1].URI}, []string{responses, responses})
	equal(t, v.Requests()[1].Header("Authorization"), "Bearer rotated-access")
}
func TestSecondRefusalFails(t *testing.T) {
	v, _, p := setup(t)
	v.RespondAt(responses, answer(401, ""))
	v.RespondAt("/oauth/token", answer(200, `{"access_token":"new-access"}`))
	v.RespondAt(responses, answer(401, "still expired"))
	f := failure(t, run(t.Context(), t, p, v.Client(), providertest.InferenceRequest()))
	equal(t, *f.Code, provider.AuthExpired)
	equal(t, f.Message, "Codex API request failed with HTTP 401: still expired")
	equal(t, len(v.Requests()), 3)
}
func TestDueTokensRefreshBeforeRequest(t *testing.T) {
	for _, test := range []struct {
		name    string
		expiry  int64
		last    core.Timestamp
		refresh bool
	}{
		{"expiring", 1789740060, now, true}, {"stale", 1789743600, "2026-09-10T14:00:00.000Z", true}, {"recent", 1789743600, "2026-09-11T14:00:01.000Z", false},
	} {
		t.Run(test.name, func(t *testing.T) {
			v := providertest.StartVendor(t)
			pool := poolWith(t, document(t, accessToken(t, test.expiry), "refresh-1", test.last))
			p := configured(t, v, pool, nil)
			v.RespondAt("/oauth/token", answer(200, `{"access_token":"new-access"}`))
			v.RespondAt(responses, completed())
			run(t.Context(), t, p, v.Client(), providertest.InferenceRequest())
			count := 1
			if test.refresh {
				count = 2
				equal(t, v.Requests()[0].URI, "/oauth/token")
				equal(t, v.Requests()[1].Header("Authorization"), "Bearer new-access")
			}
			equal(t, len(v.Requests()), count)
		})
	}
}
func TestConcurrentRequestsRefreshOnce(t *testing.T) {
	v := providertest.StartVendor(t)
	pool := poolWith(t, document(t, accessToken(t, 1789740060), "refresh-1", now))
	p := configured(t, v, pool, nil)
	v.RespondAt("/oauth/token", answer(200, `{"access_token":"new-access"}`))
	v.RespondAt(responses, completed())
	v.RespondAt(responses, completed())
	var wg sync.WaitGroup
	for range 2 {
		wg.Go(func() {
			events := run(t.Context(), t, p, v.Client(), providertest.InferenceRequest())
			if _, ok := events[len(events)-1].(*provider.Response); !ok {
				t.Errorf("events %v", events)
			}
		})
	}
	wg.Wait()
	refreshes := 0
	for _, request := range v.Requests() {
		if request.URI == "/oauth/token" {
			refreshes++
		}
		if request.URI == responses {
			equal(t, request.Header("Authorization"), "Bearer new-access")
		}
	}
	equal(t, refreshes, 1)
	equal(t, len(v.Requests()), 3)
}
func TestRefusedRefreshFails(t *testing.T) {
	v := providertest.StartVendor(t)
	pool := poolWith(t, document(t, accessToken(t, 1789740060), "refresh-1", now))
	p := configured(t, v, pool, nil)
	v.RespondAt("/oauth/token", answer(400, `{"refresh_token":"refresh-1"}`))
	f := failure(t, run(t.Context(), t, p, v.Client(), providertest.InferenceRequest()))
	equal(t, *f.Code, provider.AuthRefreshFailed)
	equal(t, f.Message, "Codex token refresh failed with HTTP 400")
}
func TestStatusAndInvalidOrMissingAccount(t *testing.T) {
	v, pool, p := setup(t)
	state, ok := p.AuthStatus(t.Context()).(*core.Authenticated)
	if !ok {
		t.Fatal("not authenticated")
	}
	equal(t, *state.AccountLabel, "dev@example.com")
	if err := pool.Write(t.Context(), provider.AccountMeta{ID: "other"}, "{}"); err != nil {
		t.Fatal(err)
	}
	if err := pool.SetActive(t.Context(), "other"); err != nil {
		t.Fatal(err)
	}
	equal(t, p.AuthStatus(t.Context()), state)
	corrupt := poolWith(t, `{"accessToken":"sk-secret-token","refreshToken":7}`)
	p = configured(t, v, corrupt, nil)
	invalid, ok := p.AuthStatus(t.Context()).(*core.AuthError)
	if !ok || strings.Contains(invalid.Message, "sk-secret-token") || !strings.Contains(invalid.Message, "refreshToken") {
		t.Fatalf("invalid state: %v", invalid)
	}
	equal(t, *failure(t, run(t.Context(), t, p, v.Client(), providertest.InferenceRequest())).Code, provider.AuthInvalid)
	p = configured(t, v, pool, func(c *codex.Config) { c.Account = nil })
	missing, ok := p.AuthStatus(t.Context()).(*core.Unauthenticated)
	if !ok {
		t.Fatal("missing account authenticated")
	}
	equal(t, *missing.Message, "No Codex account is signed in")
	equal(t, *failure(t, run(t.Context(), t, p, v.Client(), providertest.InferenceRequest())).Code, provider.AuthMissing)
	equal(t, len(v.Requests()), 0)
}
