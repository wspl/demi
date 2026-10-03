package backend_test

import (
	"bytes"
	"context"
	"net/url"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/webapi"
)

func TestASetupTokenBecomesASealedAccountThatNoAnswerReturns(t *testing.T) {
	ctx := t.Context()
	h := accountHarness(ctx, t)
	cost := provider.ProbeFree
	h.Config.Families, _ = backendtest.AccountFamilies(t, &cost)
	b, master := accountStart(ctx, t, h)
	created := accountRequest(ctx, t, b, "POST", "/api/providers/setup-token", &master, `{"token":" fixture-token-a ","label":"Claude"}`)
	accountEqual(t, created.Status, 201)
	entry := accountDecode(t, created, webapi.DecodeProviderAnswer).Provider
	accountEqual(t, entry.Kind, webapi.CredentialKindSubscription)
	accountEqual(t, entry.ProviderType, "claude-code")
	accountRefusal(t, accountRequest(ctx, t, b, "POST", "/api/providers/setup-token", &master, `{"token":"fixture-token-c","label":"Again"}`), 409, webapi.ErrorCodeProviderExists)
	path := "/api/providers/" + string(entry.ID) + "/accounts"
	added := accountRequest(ctx, t, b, "POST", path, &master, `{"token":"fixture-token-b"}`)
	accountEqual(t, added.Status, 201)
	second := accountDecode(t, added, webapi.DecodeAddedAccount).Account
	refused := accountRequest(ctx, t, b, "POST", path, &master, `{"token":"bad-token-1"}`)
	accountRefusal(t, refused, 400, webapi.ErrorCodeTokenImportFailed)
	if bytes.Contains(refused.Body, []byte("bad-token-1")) {
		t.Fatal("refusal leaks token")
	}
	listed := accountRequest(ctx, t, b, "GET", path, &master, "")
	if bytes.Contains(listed.Body, []byte("fixture-token")) {
		t.Fatal("accounts leak token")
	}
	accounts := accountDecode(t, listed, webapi.DecodeAccounts)
	accountEqual(t, len(accounts.Accounts), 2)
	if accounts.Active == nil {
		t.Fatal("no active account")
	}
	first := *accounts.Active
	if string(first) == second.ID {
		t.Fatal("added account selected implicitly")
	}
	db, err := h.ControlDatabase(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	rows, err := db.QueryContext(ctx, "SELECT secret FROM provider_credentials")
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := rows.Close(); err != nil {
			t.Error(err)
		}
	}()
	count := 0
	for rows.Next() {
		var secret []byte
		if err := rows.Scan(&secret); err != nil {
			t.Fatal(err)
		}
		if bytes.Contains(secret, []byte("fixture-token")) {
			t.Fatal("unsealed token")
		}
		count++
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
	accountEqual(t, count, 2)
	if err := rows.Close(); err != nil {
		t.Fatal(err)
	}
	active := path + "/" + string(first)
	accountRefusal(t, accountRequest(ctx, t, b, "DELETE", active, &master, ""), 409, webapi.ErrorCodeActiveAccount)
	switched := accountRequest(ctx, t, b, "PUT", path+"/active", &master, accountJSON(t, contract.Field{Name: "credentialId", Value: second.ID}))
	accountEqual(t, string(accountDecode(t, switched, webapi.DecodeActiveAccount).Active), second.ID)
	accountEqual(t, accountRequest(ctx, t, b, "DELETE", active, &master, "").Status, 204)
	accountRefusal(t, accountRequest(ctx, t, b, "PUT", path+"/active", &master, `{"credentialId":"cred-missing"}`), 404, webapi.ErrorCodeAccountNotFound)
	statusPath := "/api/providers/" + string(entry.ID) + "/status"
	status := accountRequest(ctx, t, b, "GET", statusPath, &master, "")
	if bytes.Contains(status.Body, []byte("fixture-token")) {
		t.Fatal("status leaks token")
	}
	details := accountDecode(t, status, webapi.DecodeProviderDetails)
	accountEqual(t, len(details.Accounts), 1)
	if details.Active == nil || string(*details.Active) != second.ID {
		t.Fatal(details.Active)
	}
	if err := h.AddUser(ctx, "reader@example.test", "reader-pass-1", webapi.RoleUser); err != nil {
		t.Fatal(err)
	}
	reader := accountLogin(ctx, t, b, "reader@example.test", "reader-pass-1")
	accountRefusal(t, accountRequest(ctx, t, b, "POST", path, &reader, `{"token":"not-allowed"}`), 403, webapi.ErrorCodeForbidden)
	hidden := accountDecode(t, accountRequest(ctx, t, b, "GET", path, &reader, ""), webapi.DecodeAccounts)
	accountEqual(t, len(hidden.Accounts), 0)
	if hidden.Active != nil {
		t.Fatal(hidden.Active)
	}
	seen := accountDecode(t, accountRequest(ctx, t, b, "GET", statusPath, &reader, ""), webapi.DecodeProviderDetails)
	accountEqual(t, len(seen.Accounts), 0)
	if seen.Active != nil || seen.Quota != nil {
		t.Fatal("account detail disclosed")
	}
	accountEqual[core.AuthState](t, seen.Auth, &core.Authenticated{})
}

// accountAwait observes login transitions through requests with a hang deadline, without timed sleeps.
func accountAwait(ctx context.Context, t *testing.T, b *backendtest.TestBackend, s *backendtest.Session, id string, done func(webapi.LoginState) bool) webapi.LoginState {
	t.Helper()
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	for {
		if err := ctx.Err(); err != nil {
			t.Fatal(err)
		}
		state := accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/providers/subscription-login/"+id, s, ""), webapi.DecodeLoginAnswer).Login
		if done(state) {
			return state
		}
	}
}

// accountStartLogin begins a device flow through its public route.
func accountStartLogin(ctx context.Context, t *testing.T, b *backendtest.TestBackend, s *backendtest.Session, path, body string) string {
	t.Helper()
	a := accountRequest(ctx, t, b, "POST", path, s, body)
	accountEqual(t, a.Status, 202)
	started := accountDecode(t, a, webapi.DecodeLoginStarted)
	return string(started.Login.ID)
}

// accountEnded selects terminal login observations.
func accountEnded(state webapi.LoginState) bool {
	_, pending := state.(*webapi.LoginStatePending)
	return !pending
}

// accountDeviceEntry publishes the scripted device subscription after approval.
func accountDeviceEntry(ctx context.Context, t *testing.T, b *backendtest.TestBackend, s *backendtest.Session, script *backendtest.AccountLoginScript) webapi.ProviderDTO {
	t.Helper()
	script.Approve(true)
	id := accountStartLogin(ctx, t, b, s, "/api/providers/subscription-login", `{"providerType":"device"}`)
	state := accountAwait(ctx, t, b, s, id, accountEnded)
	if _, ok := state.(*webapi.LoginStateCompleted); !ok {
		t.Fatalf("login: %#v", state)
	}
	return accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/providers", s, ""), webapi.DecodeProviders).Providers[0]
}

func TestAFreeProbeFillsTheAccountsSnapshotWhichOutlivesARestart(t *testing.T) {
	ctx := t.Context()
	h := accountHarness(ctx, t)
	cost := provider.ProbeFree
	families, script := backendtest.AccountFamilies(t, &cost)
	h.Config.Families = families
	b, master := accountStart(ctx, t, h)
	entry := accountDeviceEntry(ctx, t, b, &master, script)
	path := "/api/providers/" + string(entry.ID)
	status := accountDecode(t, accountRequest(ctx, t, b, "GET", path+"/status", &master, ""), webapi.DecodeProviderDetails)
	if status.Quota != nil {
		t.Fatal(status.Quota)
	}
	probe := webapi.ProbeCostFree
	accountEqual[webapi.QuotaCapability](t, status.QuotaCapability, &webapi.QuotaCapabilitySupported{Probe: &probe})
	snapshot := accountDecode(t, accountRequest(ctx, t, b, "POST", path+"/quota", &master, ""), webapi.DecodeQuotaAnswer).Quota
	if snapshot == nil {
		t.Fatal("no snapshot")
	}
	accountEqual(t, snapshot.Source, core.SnapshotSourceProbe)
	if snapshot.AccountLabel == nil {
		t.Fatal("no account label")
	}
	accountEqual(t, *snapshot.AccountLabel, "device@example.test")
	accountEqual(t, *snapshot.Windows[0].UsedPercent, float64(40))
	status = accountDecode(t, accountRequest(ctx, t, b, "GET", path+"/status", &master, ""), webapi.DecodeProviderDetails)
	accountEqual(t, status.Quota, snapshot)
	accountEqual(t, status.Accounts[0].Quota, snapshot)
	accountRefusal(t, accountRequest(ctx, t, b, "POST", path+"/quota", &master, `{"credentialId":"cred-missing"}`), 404, webapi.ErrorCodeAccountNotFound)
	if err := b.Close(ctx); err != nil {
		t.Fatal(err)
	}
	var err error
	b, err = h.Start(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	accountEqual(t, accountDecode(t, accountRequest(ctx, t, b, "GET", path+"/status", &master, ""), webapi.DecodeProviderDetails).Quota, snapshot)
}

func TestAProbeThatWouldSpendInferenceIsRefusedAndAnAPIKeyEntryHasNoQuota(t *testing.T) {
	ctx := t.Context()
	h := accountHarness(ctx, t)
	cost := provider.ProbeInference
	families, script := backendtest.AccountFamilies(t, &cost)
	h.Config.Families = families
	h.Config.Mode = webapi.InstanceModeIsolated
	b, master := accountStart(ctx, t, h)
	entry := accountDeviceEntry(ctx, t, b, &master, script)
	accountRefusal(t, accountRequest(ctx, t, b, "POST", "/api/providers/"+string(entry.ID)+"/quota", &master, "{}"), 409, webapi.ErrorCodeQuotaRequiresInference)
	keyed := accountDecode(t, accountRequest(ctx, t, b, "POST", "/api/providers", &master, `{"source":"custom","providerType":"anthropic","label":"Work","apiKey":"k"}`), webapi.DecodeProviderAnswer).Provider
	path := "/api/providers/" + string(keyed.ID)
	quota := accountDecode(t, accountRequest(ctx, t, b, "POST", path+"/quota", &master, "{}"), webapi.DecodeQuotaAnswer)
	if quota.Quota != nil {
		t.Fatal(quota.Quota)
	}
	status := accountDecode(t, accountRequest(ctx, t, b, "GET", path+"/status", &master, ""), webapi.DecodeProviderDetails)
	accountEqual[webapi.QuotaCapability](t, status.QuotaCapability, &webapi.QuotaCapabilityNone{})
	accountEqual(t, len(status.Accounts), 0)
}

func TestConcurrentDeviceLoginsPublishOneEntryAndTheOtherStoresNothing(t *testing.T) {
	ctx := t.Context()
	h := accountHarness(ctx, t)
	cost := provider.ProbeFree
	families, script := backendtest.AccountFamilies(t, &cost)
	h.Config.Families = families
	vendor := providertest.StartVendor(t)
	vendor.RespondAt("/api.json", providertest.MockResponse{Status: 200, Chunks: [][]byte{[]byte("{}")}})
	var err error
	h.Config.ModelsDevURL, err = url.Parse(vendor.URL("/api.json"))
	if err != nil {
		t.Fatal(err)
	}
	b, master := accountStart(ctx, t, h)
	path := "/api/providers/subscription-login"
	for _, row := range []struct {
		body string
		code webapi.ErrorCode
	}{{`{"providerType":"nope"}`, webapi.ErrorCodeUnknownProviderType}, {`{"providerType":"anthropic"}`, webapi.ErrorCodeNoLoginFlow}, {`{"type":"device"}`, webapi.ErrorCodeInvalidBody}} {
		accountRefusal(t, accountRequest(ctx, t, b, "POST", path, &master, row.body), 400, row.code)
	}
	first := accountStartLogin(ctx, t, b, &master, path, `{"providerType":"device","label":"Work"}`)
	second := accountStartLogin(ctx, t, b, &master, path, `{"providerType":"device","label":"Competing"}`)
	pending := accountAwait(ctx, t, b, &master, first, func(s webapi.LoginState) bool {
		p, ok := s.(*webapi.LoginStatePending)
		return ok && p.VerificationURL != nil
	})
	verification, code := "https://verify.example/device", "ABCD-1234"
	accountEqual[webapi.LoginState](t, pending, &webapi.LoginStatePending{VerificationURL: &verification, UserCode: &code})
	script.Approve(true)
	outcomes := []webapi.LoginState{accountAwait(ctx, t, b, &master, first, accountEnded), accountAwait(ctx, t, b, &master, second, accountEnded)}
	var completed *webapi.LoginStateCompleted
	successes := 0
	failed := false
	for _, state := range outcomes {
		switch s := state.(type) {
		case *webapi.LoginStateCompleted:
			completed = s
			successes++
		case *webapi.LoginStateFailed:
			failed = s.Message == "This scope already has a device subscription"
		case *webapi.LoginStatePending:
			t.Fatal("pending")
		}
	}
	accountEqual(t, successes, 1)
	accountEqual(t, failed, true)
	entries := accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/providers", &master, ""), webapi.DecodeProviders).Providers
	accountEqual(t, len(entries), 1)
	accountEqual(t, entries[0].ID, completed.ProviderID)
	accounts := accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/providers/"+string(completed.ProviderID)+"/accounts", &master, ""), webapi.DecodeAccounts)
	accountEqual(t, len(accounts.Accounts), 1)
	accountEqual(t, accounts.Accounts[0].ID, string(completed.CredentialID))
	accountEqual(t, *accounts.Active, completed.CredentialID)
	db, err := h.ControlDatabase(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	var count int
	var secret []byte
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*), MAX(secret) FROM provider_credentials").Scan(&count, &secret); err != nil {
		t.Fatal(err)
	}
	accountEqual(t, count, 1)
	if bytes.Contains(secret, []byte("login-secret")) {
		t.Fatal("unsealed login")
	}
	offered := accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/providers/catalog", &master, ""), webapi.DecodeVendorCatalog)
	configured := false
	for _, family := range offered.Subscriptions {
		if family.ProviderType == "device" && family.Configured {
			configured = true
		}
	}
	accountEqual(t, configured, true)
	accountRefusal(t, accountRequest(ctx, t, b, "POST", path, &master, `{"providerType":"device"}`), 409, webapi.ErrorCodeProviderExists)
	path = "/api/providers/" + string(completed.ProviderID)
	accountEqual(t, accountDecode(t, accountRequest(ctx, t, b, "PATCH", path, &master, `{"label":"Personal"}`), webapi.DecodeProviderAnswer).Provider.Label, "Personal")
	accountRefusal(t, accountRequest(ctx, t, b, "PATCH", path, &master, `{"apiKey":"k"}`), 400, webapi.ErrorCodeSubscriptionOnly)
	accountEqual(t, accountRequest(ctx, t, b, "DELETE", path, &master, "").Status, 204)
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM provider_credentials").Scan(&count); err != nil {
		t.Fatal(err)
	}
	accountEqual(t, count, 0)
}

func TestALoginIntoAnEntryHoldsItUntilItEndsAndCancellingStopsItAtOnce(t *testing.T) {
	ctx := t.Context()
	h := accountHarness(ctx, t)
	cost := provider.ProbeFree
	families, script := backendtest.AccountFamilies(t, &cost)
	h.Config.Families = families
	b, master := accountStart(ctx, t, h)
	entry := accountDeviceEntry(ctx, t, b, &master, script)
	script.Approve(false)
	path := "/api/providers/" + string(entry.ID)
	id := accountStartLogin(ctx, t, b, &master, path+"/accounts/login", "{}")
	accountRefusal(t, accountRequest(ctx, t, b, "PATCH", path, &master, `{"label":"Busy"}`), 409, webapi.ErrorCodeProviderBusy)
	accountRefusal(t, accountRequest(ctx, t, b, "DELETE", path, &master, ""), 409, webapi.ErrorCodeProviderBusy)
	accountEqual(t, accountRequest(ctx, t, b, "DELETE", "/api/providers/subscription-login/"+id, &master, "").Status, 204)
	accountEqual(t, script.Cancelled.Load(), int64(1))
	accountEqual[webapi.LoginState](t, accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/providers/subscription-login/"+id, &master, ""), webapi.DecodeLoginAnswer).Login, &webapi.LoginStateFailed{Message: "The login was cancelled"})
	accountEqual(t, accountRequest(ctx, t, b, "PATCH", path, &master, `{"label":"Ready"}`).Status, 200)
	accountRefusal(t, accountRequest(ctx, t, b, "DELETE", "/api/providers/subscription-login/no-such-login", &master, ""), 404, webapi.ErrorCodeLoginNotFound)
}

func TestALoginExpiresAndItsResultGoesAfterTheRetention(t *testing.T) {
	ctx, cancel := context.WithTimeout(t.Context(), 10*time.Second)
	defer cancel()
	h := accountHarness(ctx, t)
	cost := provider.ProbeFree
	families, script := backendtest.AccountFamilies(t, &cost)
	h.Config.Families = families
	timing := providers.LoginTiming{Lifetime: 100 * time.Millisecond, Retention: 300 * time.Millisecond}
	h.Config.Logins = timing
	b, master := accountStart(ctx, t, h)
	started := time.Now()
	id := accountStartLogin(ctx, t, b, &master, "/api/providers/subscription-login", `{"providerType":"device"}`)
	accountEqual[webapi.LoginState](t, accountAwait(ctx, t, b, &master, id, accountEnded), &webapi.LoginStateFailed{Message: "The login expired"})
	accountEqual(t, script.Cancelled.Load(), int64(1))
	accountEqual(t, len(accountDecode(t, accountRequest(ctx, t, b, "GET", "/api/providers", &master, ""), webapi.DecodeProviders).Providers), 0)
	path := "/api/providers/subscription-login/" + id
	for {
		a := accountRequest(ctx, t, b, "GET", path, &master, "")
		if a.Status == 404 {
			break
		}
		if err := ctx.Err(); err != nil {
			t.Fatal(err)
		}
	}
	if time.Since(started) < timing.Lifetime+timing.Retention {
		t.Fatal("login result removed before retention")
	}
	accountRefusal(t, accountRequest(ctx, t, b, "GET", path, &master, ""), 404, webapi.ErrorCodeLoginNotFound)
}
