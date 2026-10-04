package scenarios_test

import (
	"bytes"
	"context"
	"net/url"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/providerhost"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// TestASetupTokenBecomesASealedAccountThatNoAnswerReturns
// checks account sealing and credential disclosure boundaries.
func TestASetupTokenBecomesASealedAccountThatNoAnswerReturns(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	cost := provider.ProbeFree
	harness.Config.Families, _ = backendtest.AccountFamilies(t, &cost)
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	created := conversationRequest(
		ctx,
		t,
		server,
		&master,
		"POST",
		"/api/providers/setup-token",
		`{"token":" fixture-token-a ","label":"Claude"}`,
		201,
	)
	entry := conversationDecode(t, created, webapiproto.DecodeProviderAnswer).Provider
	conversationEqual(t, entry.Kind, webapiproto.CredentialKindSubscription)
	conversationEqual(t, entry.ProviderType, "claude-code")
	conversationRefusal(
		t,
		conversationRequest(
			ctx,
			t,
			server,
			&master,
			"POST",
			"/api/providers/setup-token",
			`{"token":"fixture-token-c","label":"Again"}`,
			409,
		),
		webapiproto.ErrorCodeProviderExists,
	)
	path := "/api/providers/" + string(entry.ID) + "/accounts"
	added := conversationRequest(ctx, t, server, &master, "POST", path, `{"token":"fixture-token-b"}`, 201)
	second := conversationDecode(t, added, webapiproto.DecodeAddedAccount).Account
	refused := conversationRequest(ctx, t, server, &master, "POST", path, `{"token":"bad-token-1"}`, 400)
	conversationRefusal(t, refused, webapiproto.ErrorCodeTokenImportFailed)
	if bytes.Contains(refused.Body, []byte("bad-token-1")) {
		t.Fatal("refusal leaks token")
	}
	listed := conversationRequest(ctx, t, server, &master, "GET", path, "", 200)
	if bytes.Contains(listed.Body, []byte("fixture-token")) {
		t.Fatal("accounts leak token")
	}
	accounts := conversationDecode(t, listed, webapiproto.DecodeAccounts)
	conversationEqual(t, len(accounts.Accounts), 2)
	if accounts.Active == nil {
		t.Fatal("no active account")
	}
	first := *accounts.Active
	if string(first) == second.ID {
		t.Fatal("added account selected implicitly")
	}
	db, err := harness.ControlDatabase(ctx, t)
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
	conversationEqual(t, count, 2)
	if err := rows.Close(); err != nil {
		t.Fatal(err)
	}
	active := path + "/" + string(first)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, server, &master, "DELETE", active, "", 409),
		webapiproto.ErrorCodeActiveAccount,
	)
	switched := conversationRequest(
		ctx,
		t,
		server,
		&master,
		"PUT",
		path+"/active",
		accountJSON(t, contract.Field{Name: "credentialId", Value: second.ID}),
		200,
	)
	conversationEqual(t, string(conversationDecode(t, switched, webapiproto.DecodeActiveAccount).Active), second.ID)
	conversationRequest(ctx, t, server, &master, "DELETE", active, "", 204)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, server, &master, "PUT", path+"/active", `{"credentialId":"cred-missing"}`, 404),
		webapiproto.ErrorCodeAccountNotFound,
	)
	statusPath := "/api/providers/" + string(entry.ID) + "/status"
	status := conversationRequest(ctx, t, server, &master, "GET", statusPath, "", 200)
	if bytes.Contains(status.Body, []byte("fixture-token")) {
		t.Fatal("status leaks token")
	}
	details := conversationDecode(t, status, webapiproto.DecodeProviderDetails)
	conversationEqual(t, len(details.Accounts), 1)
	if details.Active == nil || string(*details.Active) != second.ID {
		t.Fatal(details.Active)
	}
	if err := harness.AddUser(ctx, "reader@example.test", "reader-pass-1", webapiproto.RoleUser); err != nil {
		t.Fatal(err)
	}
	reader, err := server.Login(ctx, "reader@example.test", "reader-pass-1")
	wireMust(t, err)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, server, &reader, "POST", path, `{"token":"not-allowed"}`, 403),
		webapiproto.ErrorCodeForbidden,
	)
	hidden := conversationDecode(
		t,
		conversationRequest(ctx, t, server, &reader, "GET", path, "", 200),
		webapiproto.DecodeAccounts,
	)
	conversationEqual(t, len(hidden.Accounts), 0)
	if hidden.Active != nil {
		t.Fatal(hidden.Active)
	}
	seen := conversationDecode(
		t,
		conversationRequest(ctx, t, server, &reader, "GET", statusPath, "", 200),
		webapiproto.DecodeProviderDetails,
	)
	conversationEqual(t, len(seen.Accounts), 0)
	if seen.Active != nil || seen.Quota != nil {
		t.Fatal("account detail disclosed")
	}
	conversationEqual[types.AuthState](t, seen.Auth, &types.Authenticated{})
}

// accountAwait observes login transitions through requests; the package timeout guards against a hang.
func accountAwait(
	ctx context.Context,
	t *testing.T,
	server *backendtest.TestBackend,
	s *backendtest.Session,
	id string,
	done func(webapiproto.LoginState) bool,
) webapiproto.LoginState {
	t.Helper()
	for {
		if err := ctx.Err(); err != nil {
			t.Fatal(err)
		}
		state := conversationDecode(
			t,
			conversationRequest(ctx, t, server, s, "GET", "/api/providers/subscription-login/"+id, "", 200),
			webapiproto.DecodeLoginAnswer,
		).Login
		if done(state) {
			return state
		}
	}
}

// accountStartLogin begins a device flow through its public route.
func accountStartLogin(
	ctx context.Context,
	t *testing.T,
	server *backendtest.TestBackend,
	s *backendtest.Session,
	path, body string,
) string {
	t.Helper()
	a := conversationRequest(ctx, t, server, s, "POST", path, body, 202)
	started := conversationDecode(t, a, webapiproto.DecodeLoginStarted)
	return string(started.Login.ID)
}

// accountEnded selects terminal login observations.
func accountEnded(state webapiproto.LoginState) bool {
	_, pending := state.(*webapiproto.LoginStatePending)
	return !pending
}

// accountDeviceEntry publishes the scripted device subscription after approval.
func accountDeviceEntry(
	ctx context.Context,
	t *testing.T,
	server *backendtest.TestBackend,
	s *backendtest.Session,
	script *backendtest.AccountLoginScript,
) webapiproto.ProviderDTO {
	t.Helper()
	script.SetApproved(true)
	id := accountStartLogin(ctx, t, server, s, "/api/providers/subscription-login", `{"providerType":"device"}`)
	state := accountAwait(ctx, t, server, s, id, accountEnded)
	if _, ok := state.(*webapiproto.LoginStateCompleted); !ok {
		t.Fatalf("login: %#v", state)
	}
	return conversationDecode(
		t,
		conversationRequest(ctx, t, server, s, "GET", "/api/providers", "", 200),
		webapiproto.DecodeProviders,
	).Providers[0]
}

// TestAFreeProbeFillsTheAccountsSnapshotWhichOutlivesARestart
// checks quota observations and their restart persistence.
func TestAFreeProbeFillsTheAccountsSnapshotWhichOutlivesARestart(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	cost := provider.ProbeFree
	families, script := backendtest.AccountFamilies(t, &cost)
	harness.Config.Families = families
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	entry := accountDeviceEntry(ctx, t, server, &master, script)
	path := "/api/providers/" + string(entry.ID)
	status := conversationDecode(
		t,
		conversationRequest(ctx, t, server, &master, "GET", path+"/status", "", 200),
		webapiproto.DecodeProviderDetails,
	)
	if status.Quota != nil {
		t.Fatal(status.Quota)
	}
	probe := webapiproto.ProbeCostFree
	conversationEqual[webapiproto.QuotaCapability](
		t,
		status.QuotaCapability,
		&webapiproto.QuotaCapabilitySupported{Probe: &probe},
	)
	snapshot := conversationDecode(
		t,
		conversationRequest(ctx, t, server, &master, "POST", path+"/quota", "", 200),
		webapiproto.DecodeQuotaAnswer,
	).Quota
	if snapshot == nil {
		t.Fatal("no snapshot")
	}
	conversationEqual(t, snapshot.Source, types.SnapshotSourceProbe)
	if snapshot.AccountLabel == nil {
		t.Fatal("no account label")
	}
	conversationEqual(t, *snapshot.AccountLabel, "device@example.test")
	conversationEqual(t, *snapshot.Windows[0].UsedPercent, float64(40))
	status = conversationDecode(
		t,
		conversationRequest(ctx, t, server, &master, "GET", path+"/status", "", 200),
		webapiproto.DecodeProviderDetails,
	)
	conversationEqual(t, status.Quota, snapshot)
	conversationEqual(t, status.Accounts[0].Quota, snapshot)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, server, &master, "POST", path+"/quota", `{"credentialId":"cred-missing"}`, 404),
		webapiproto.ErrorCodeAccountNotFound,
	)
	if err := server.Close(ctx); err != nil {
		t.Fatal(err)
	}
	server, err = harness.Start(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	conversationEqual(
		t,
		conversationDecode(
			t,
			conversationRequest(ctx, t, server, &master, "GET", path+"/status", "", 200),
			webapiproto.DecodeProviderDetails,
		).Quota,
		snapshot,
	)
}

// TestAProbeThatWouldSpendInferenceIsRefusedAndAnAPIKeyEntryHasNoQuota
// checks which account entries permit quota probes.
func TestAProbeThatWouldSpendInferenceIsRefusedAndAnAPIKeyEntryHasNoQuota(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	cost := provider.ProbeInference
	families, script := backendtest.AccountFamilies(t, &cost)
	harness.Config.Families = families
	harness.Config.Mode = webapiproto.InstanceModeIsolated
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	entry := accountDeviceEntry(ctx, t, server, &master, script)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, server, &master, "POST", "/api/providers/"+string(entry.ID)+"/quota", "{}", 409),
		webapiproto.ErrorCodeQuotaRequiresInference,
	)
	keyed := conversationDecode(
		t,
		conversationRequest(
			ctx,
			t,
			server,
			&master,
			"POST",
			"/api/providers",
			`{"source":"custom","providerType":"anthropic","label":"Work","apiKey":"k"}`,
			201,
		),
		webapiproto.DecodeProviderAnswer,
	).Provider
	path := "/api/providers/" + string(keyed.ID)
	quota := conversationDecode(
		t,
		conversationRequest(ctx, t, server, &master, "POST", path+"/quota", "{}", 200),
		webapiproto.DecodeQuotaAnswer,
	)
	if quota.Quota != nil {
		t.Fatal(quota.Quota)
	}
	status := conversationDecode(
		t,
		conversationRequest(ctx, t, server, &master, "GET", path+"/status", "", 200),
		webapiproto.DecodeProviderDetails,
	)
	conversationEqual[webapiproto.QuotaCapability](t, status.QuotaCapability, &webapiproto.QuotaCapabilityNone{})
	conversationEqual(t, len(status.Accounts), 0)
}

// TestConcurrentDeviceLoginsPublishOneEntryAndTheOtherStoresNothing
// checks publication when device logins compete.
func TestConcurrentDeviceLoginsPublishOneEntryAndTheOtherStoresNothing(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	cost := provider.ProbeFree
	families, script := backendtest.AccountFamilies(t, &cost)
	harness.Config.Families = families
	vendor := providertest.StartVendor(t)
	vendor.RespondAt("/api.json", providertest.MockResponse{Status: 200, Chunks: [][]byte{[]byte("{}")}})
	var err error
	harness.Config.ModelsDevURL, err = url.Parse(vendor.URL("/api.json"))
	if err != nil {
		t.Fatal(err)
	}
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	path := "/api/providers/subscription-login"
	for _, row := range []struct {
		body string
		code webapiproto.ErrorCode
	}{
		{`{"providerType":"nope"}`, webapiproto.ErrorCodeUnknownProviderType},
		{`{"providerType":"anthropic"}`, webapiproto.ErrorCodeNoLoginFlow},
		{`{"type":"device"}`, webapiproto.ErrorCodeInvalidBody},
	} {
		conversationRefusal(t, conversationRequest(ctx, t, server, &master, "POST", path, row.body, 400), row.code)
	}
	first := accountStartLogin(ctx, t, server, &master, path, `{"providerType":"device","label":"Work"}`)
	second := accountStartLogin(ctx, t, server, &master, path, `{"providerType":"device","label":"Competing"}`)
	pending := accountAwait(ctx, t, server, &master, first, func(s webapiproto.LoginState) bool {
		p, ok := s.(*webapiproto.LoginStatePending)
		return ok && p.VerificationURL != nil
	})
	verification, code := "https://verify.example/device", "ABCD-1234"
	conversationEqual[webapiproto.LoginState](
		t,
		pending,
		&webapiproto.LoginStatePending{VerificationURL: &verification, UserCode: &code},
	)
	script.SetApproved(true)
	outcomes := []webapiproto.LoginState{
		accountAwait(ctx, t, server, &master, first, accountEnded),
		accountAwait(ctx, t, server, &master, second, accountEnded),
	}
	var completed *webapiproto.LoginStateCompleted
	successes := 0
	failed := false
	for _, state := range outcomes {
		switch s := state.(type) {
		case *webapiproto.LoginStateCompleted:
			completed = s
			successes++
		case *webapiproto.LoginStateFailed:
			failed = s.Message == "This scope already has a device subscription"
		case *webapiproto.LoginStatePending:
			t.Fatal("pending")
		}
	}
	conversationEqual(t, successes, 1)
	conversationEqual(t, failed, true)
	entries := conversationDecode(
		t,
		conversationRequest(ctx, t, server, &master, "GET", "/api/providers", "", 200),
		webapiproto.DecodeProviders,
	).Providers
	conversationEqual(t, len(entries), 1)
	conversationEqual(t, entries[0].ID, completed.ProviderID)
	accounts := conversationDecode(
		t,
		conversationRequest(
			ctx,
			t,
			server,
			&master,
			"GET",
			"/api/providers/"+string(completed.ProviderID)+"/accounts",
			"",
			200,
		),
		webapiproto.DecodeAccounts,
	)
	conversationEqual(t, len(accounts.Accounts), 1)
	conversationEqual(t, accounts.Accounts[0].ID, string(completed.CredentialID))
	conversationEqual(t, *accounts.Active, completed.CredentialID)
	db, err := harness.ControlDatabase(ctx, t)
	if err != nil {
		t.Fatal(err)
	}
	var count int
	var secret []byte
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*), MAX(secret) FROM provider_credentials").
		Scan(&count, &secret); err != nil {
		t.Fatal(err)
	}
	conversationEqual(t, count, 1)
	if bytes.Contains(secret, []byte("login-secret")) {
		t.Fatal("unsealed login")
	}
	offered := conversationDecode(
		t,
		conversationRequest(ctx, t, server, &master, "GET", "/api/providers/catalog", "", 200),
		webapiproto.DecodeVendorCatalog,
	)
	configured := false
	for _, family := range offered.Subscriptions {
		if family.ProviderType == "device" && family.Configured {
			configured = true
		}
	}
	conversationEqual(t, configured, true)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, server, &master, "POST", path, `{"providerType":"device"}`, 409),
		webapiproto.ErrorCodeProviderExists,
	)
	path = "/api/providers/" + string(completed.ProviderID)
	conversationEqual(
		t,
		conversationDecode(
			t,
			conversationRequest(ctx, t, server, &master, "PATCH", path, `{"label":"Personal"}`, 200),
			webapiproto.DecodeProviderAnswer,
		).Provider.Label,
		"Personal",
	)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, server, &master, "PATCH", path, `{"apiKey":"k"}`, 400),
		webapiproto.ErrorCodeSubscriptionOnly,
	)
	conversationRequest(ctx, t, server, &master, "DELETE", path, "", 204)
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM provider_credentials").Scan(&count); err != nil {
		t.Fatal(err)
	}
	conversationEqual(t, count, 0)
}

// TestALoginIntoAnEntryHoldsItUntilItEndsAndCancellingStopsItAtOnce
// checks login reservations and cancellation.
func TestALoginIntoAnEntryHoldsItUntilItEndsAndCancellingStopsItAtOnce(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	cost := provider.ProbeFree
	families, script := backendtest.AccountFamilies(t, &cost)
	harness.Config.Families = families
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	entry := accountDeviceEntry(ctx, t, server, &master, script)
	script.SetApproved(false)
	path := "/api/providers/" + string(entry.ID)
	id := accountStartLogin(ctx, t, server, &master, path+"/accounts/login", "{}")
	conversationRefusal(
		t,
		conversationRequest(ctx, t, server, &master, "PATCH", path, `{"label":"Busy"}`, 409),
		webapiproto.ErrorCodeProviderBusy,
	)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, server, &master, "DELETE", path, "", 409),
		webapiproto.ErrorCodeProviderBusy,
	)
	conversationRequest(ctx, t, server, &master, "DELETE", "/api/providers/subscription-login/"+id, "", 204)
	conversationEqual(t, script.Cancelled.Load(), int64(1))
	conversationEqual[webapiproto.LoginState](
		t,
		conversationDecode(
			t,
			conversationRequest(ctx, t, server, &master, "GET", "/api/providers/subscription-login/"+id, "", 200),
			webapiproto.DecodeLoginAnswer,
		).Login,
		&webapiproto.LoginStateFailed{Message: "The login was cancelled"},
	)
	conversationRequest(ctx, t, server, &master, "PATCH", path, `{"label":"Ready"}`, 200)
	conversationRefusal(
		t,
		conversationRequest(
			ctx,
			t,
			server,
			&master,
			"DELETE",
			"/api/providers/subscription-login/no-such-login",
			"",
			404,
		),
		webapiproto.ErrorCodeLoginNotFound,
	)
}

// TestALoginExpiresAndItsResultGoesAfterTheRetention
// checks login expiry and terminal-result retention.
func TestALoginExpiresAndItsResultGoesAfterTheRetention(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	cost := provider.ProbeFree
	families, script := backendtest.AccountFamilies(t, &cost)
	harness.Config.Families = families
	timing := providerhost.LoginTiming{Lifetime: 100 * time.Millisecond, Retention: 300 * time.Millisecond}
	harness.Config.Logins = timing
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	started := time.Now()
	id := accountStartLogin(ctx, t, server, &master, "/api/providers/subscription-login", `{"providerType":"device"}`)
	conversationEqual[webapiproto.LoginState](
		t,
		accountAwait(ctx, t, server, &master, id, accountEnded),
		&webapiproto.LoginStateFailed{Message: "The login expired"},
	)
	conversationEqual(t, script.Cancelled.Load(), int64(1))
	conversationEqual(
		t,
		len(
			conversationDecode(
				t,
				conversationRequest(ctx, t, server, &master, "GET", "/api/providers", "", 200),
				webapiproto.DecodeProviders,
			).Providers,
		),
		0,
	)
	path := "/api/providers/subscription-login/" + id
	for {
		a, err := server.Read(ctx, path, &master)
		wireMust(t, err)
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
	conversationRefusal(
		t,
		conversationRequest(ctx, t, server, &master, "GET", path, "", 404),
		webapiproto.ErrorCodeLoginNotFound,
	)
}
