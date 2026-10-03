package grokbuild

import (
	"net/url"
	"strings"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func Test401RefreshPrincipal(t *testing.T) {
	v := providertest.StartVendor(t)
	v.RespondAt(chatPath, answer(401, "unauthorized"))
	v.RespondAt(
		"/oauth2/token",
		answer(
			200,
			`{"access_token":"refreshed-token","refresh_token":"refresh-2",`+
				`"expires_in":3600,"token_type":"Bearer"}`,
		),
	)
	v.RespondAt(chatPath, chat(`{"choices":[{"delta":{"content":"ok"}}]}`))
	p, pool := fixture(t, v, map[string]any{"principal": map[string]any{"kind": "User", "id": "user-1"}})
	equal(
		t,
		[]provider.Event{&provider.TextDelta{Text: "ok"}, &provider.Response{}},
		run(t, p, providertest.InferenceRequest()),
	)
	requests := v.Requests()
	equal(t, 3, len(requests))
	equal(t, chatPath, requests[0].URI)
	equal(t, "/oauth2/token", requests[1].URI)
	equal(t, chatPath, requests[2].URI)
	form, err := url.ParseQuery(string(requests[1].Body))
	if err != nil {
		t.Fatal(err)
	}
	equal(
		t,
		url.Values{
			"grant_type":     {"refresh_token"},
			"refresh_token":  {"refresh-1"},
			"client_id":      {"client-1"},
			"principal_type": {"User"},
			"principal_id":   {"user-1"},
		},
		form,
	)
	equal(t, "Bearer refreshed-token", requests[2].Header("authorization"))
	equal(t, string(requests[0].Body), string(requests[2].Body))
	s := stored(t, pool)
	equal(t, "refreshed-token", s.AccessToken.Expose())
	equal(t, "refresh-2", s.RefreshToken.Expose())
	equal(t, core.Timestamp("2026-09-18T15:00:00.000Z"), *s.ExpiresAt)
	equal(t, "user@example.com", *s.Email)
}

func TestConcurrentReplacementAvoidsRefresh(t *testing.T) {
	v := providertest.StartVendor(t)
	v.RespondAt(chatPath, answer(401, ""))
	v.RespondAt(chatPath, chat())
	p, pool := fixture(t, v, nil)
	doc := pool.Document("cred-g")
	turn, err := doc.RefreshTurn(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	defer turn.Release()
	done := make(chan []provider.Event, 1)
	finished := make(chan struct{})
	go func() {
		defer close(finished)
		defer close(done)
		done <- run(t, p, providertest.InferenceRequest())
	}()
	t.Cleanup(func() { <-finished })
	v.Received(t.Context(), 1)
	entry, _, err := doc.Read(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	rotated := strings.ReplaceAll(entry.Text, "session-token", "rotated-token")
	kept, err := doc.Replace(t.Context(), rotated, entry.Version)
	if err != nil {
		t.Fatal(err)
	}
	equal(t, true, kept)
	turn.Release()
	equal(t, []provider.Event{&provider.Response{}}, <-done)
	equal(t, 2, len(v.Requests()))
	for _, request := range v.Requests() {
		equal(t, chatPath, request.URI)
	}
	equal(t, "Bearer rotated-token", v.Requests()[1].Header("authorization"))
}

func TestExpiringAndUnrefreshableTokens(t *testing.T) {
	for _, refresh := range []bool{true, false} {
		t.Run(map[bool]string{true: "refresh", false: "no refresh token"}[refresh], func(t *testing.T) {
			v := providertest.StartVendor(t)
			fields := map[string]any{"expiresAt": "2026-09-18T14:01:00.000Z"}
			if refresh {
				v.RespondAt("/oauth2/token", answer(200, `{"access_token":"fresh","expires_in":"3600"}`))
			} else {
				fields["refreshToken"] = nil
			}
			v.RespondAt(chatPath, chat())
			p, pool := fixture(t, v, fields)
			equal(t, []provider.Event{&provider.Response{}}, run(t, p, providertest.InferenceRequest()))
			if refresh {
				equal(t, 2, len(v.Requests()))
				equal(t, "/oauth2/token", v.Requests()[0].URI)
				equal(t, "Bearer fresh", v.Requests()[1].Header("authorization"))
				equal(t, "refresh-1", stored(t, pool).RefreshToken.Expose())
			} else {
				equal(t, 1, len(v.Requests()))
				equal(t, "Bearer session-token", v.Requests()[0].Header("authorization"))
			}
		})
	}
}

func TestConcurrentRequestsRefreshOnce(t *testing.T) {
	v := providertest.StartVendor(t)
	v.RespondAt("/oauth2/token", answer(200, `{"access_token":"fresh","expires_in":3600}`))
	v.RespondAt(chatPath, chat())
	v.RespondAt(chatPath, chat())
	p, _ := fixture(t, v, map[string]any{"expiresAt": "2026-09-18T14:01:00.000Z"})
	var wg sync.WaitGroup
	for range 2 {
		wg.Go(func() { equal(t, []provider.Event{&provider.Response{}}, run(t, p, providertest.InferenceRequest())) })
	}
	wg.Wait()
	requests := v.Requests()
	equal(t, 3, len(requests))
	refreshes := 0
	for _, r := range requests {
		if r.URI == "/oauth2/token" {
			refreshes++
		} else {
			equal(t, "Bearer fresh", r.Header("authorization"))
		}
	}
	equal(t, 1, refreshes)
}

func TestFailedRefresh(t *testing.T) {
	for _, tc := range []struct {
		name          string
		status        int
		body, message string
	}{{
		"refused",
		400,
		`{"error":"invalid_grant"}`,
		"Grok token refresh failed with HTTP 400",
	}, {
		"missing access",
		200,
		`{"token_type":"Bearer","expires_in":3600}`,
		"Grok token refresh failed: the response is malformed",
	}} {
		t.Run(tc.name, func(t *testing.T) {
			v := providertest.StartVendor(t)
			v.RespondAt("/oauth2/token", answer(tc.status, tc.body))
			p, _ := fixture(t, v, map[string]any{"expiresAt": "2026-09-18T14:01:00.000Z"})
			events := run(t, p, providertest.InferenceRequest())
			equal(t, 1, len(events))
			failure := events[0].(*provider.Error).Failure
			equal(t, provider.AuthRefreshFailed, *failure.Code)
			if tc.status == 400 {
				equal(t, tc.message, failure.Message)
			} else if !strings.HasPrefix(failure.Message, tc.message) {
				t.Fatal(failure.Message)
			}
			equal(t, 1, len(v.Requests()))
			equal(t, "/oauth2/token", v.Requests()[0].URI)
		})
	}
}

func TestAccountStatus(t *testing.T) {
	v := providertest.StartVendor(t)
	p, pool := fixture(t, v, nil)
	name := "user@example.com"
	want := &core.Authenticated{AccountLabel: &name}
	equal(t, core.AuthState(want), p.AuthStatus(t.Context()))
	if err := pool.Write(
		t.Context(),
		provider.AccountMeta{ID: "other", Label: "other", UpdatedAt: now},
		"{}",
	); err != nil {
		t.Fatal(err)
	}
	if err := pool.SetActive(t.Context(), "other"); err != nil {
		t.Fatal(err)
	}
	equal(t, core.AuthState(want), p.AuthStatus(t.Context()))
	corrupt, _ := fixture(t, v, map[string]any{"refreshToken": 7, "accessToken": "sk-secret-token"})
	state, ok := corrupt.AuthStatus(t.Context()).(*core.AuthError)
	if !ok {
		t.Fatal("corrupt account usable")
	}
	if !strings.Contains(state.Message, "refreshToken") || strings.Contains(state.Message, "sk-secret-token") {
		t.Fatal(state.Message)
	}
	events := run(t, corrupt, providertest.InferenceRequest())
	equal(t, 1, len(events))
	equal(t, provider.AuthInvalid, *events[0].(*provider.Error).Failure.Code)
	staged := testProvider(v, provider.NewMemoryCredentialPool(), nil, v.Client())
	message := "No Grok account is signed in"
	equal(t, core.AuthState(&core.Unauthenticated{Message: &message}), staged.AuthStatus(t.Context()))
	events = run(t, staged, providertest.InferenceRequest())
	equal(t, 1, len(events))
	equal(t, provider.AuthMissing, *events[0].(*provider.Error).Failure.Code)
	equal(t, 0, len(v.Requests()))
}
