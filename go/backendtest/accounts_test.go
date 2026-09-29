package backendtest_test

import (
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/scripted"
)

// The subscription entries run on the backend's real families: the codex
// family's device login and quota probe against a scripted Codex, the
// claude-code family's setup tokens against its own code, with the endpoints
// pointed at the suite's servers through the tuning file.

const loginRoute = "/api/providers/subscription-login"

// startLogin starts a login at path and answers its id.
func startLogin(t *testing.T, b *backendtest.Backend, session *backendtest.Session, path string, body backendtest.Map) string {
	answer := b.Post(path, session, body).Expect(http.StatusAccepted)
	if answer.Str("login.status") != "pending" {
		t.Fatalf("a login starts as %s", answer.Body)
	}
	return answer.Str("login.id")
}

// loginState is the login as its route reads it.
func loginState(b *backendtest.Backend, session *backendtest.Session, id string) map[string]any {
	state, _ := b.Get(loginRoute+"/"+id, session).Expect(http.StatusOK).At("login").(map[string]any)
	return state
}

// untilLogin returns the login's state once settled accepts it.
func untilLogin(t *testing.T, b *backendtest.Backend, session *backendtest.Session, id string, settled func(map[string]any) bool) map[string]any {
	t.Helper()
	var state map[string]any
	backendtest.Eventually(t, "the login settles", func() bool {
		state = loginState(b, session, id)
		return settled(state)
	})
	return state
}

func loginEnded(state map[string]any) bool { return state["status"] != "pending" }

// codexEntry is an entry of the codex family with one account, from a login the
// scripted Codex approves.
func codexEntry(t *testing.T, b *backendtest.Backend, session *backendtest.Session, codex *scripted.Codex) map[string]any {
	t.Helper()
	codex.Approve(true)
	id := startLogin(t, b, session, loginRoute, backendtest.Map{"providerType": "codex"})
	state := untilLogin(t, b, session, id, loginEnded)
	if state["status"] != "completed" {
		t.Fatalf("the login is %v", state)
	}
	entries := providerList(b, session)
	if len(entries) != 1 {
		t.Fatalf("the entries are %v", entries)
	}
	return entries[0].(map[string]any)
}

// Cost: one backend, about a second.
func TestASetupTokenBecomesASealedAccountThatNoAnswerReturns(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t)
	b, master := h.StartSetUp()
	created := b.Post("/api/providers/setup-token", master, backendtest.Map{"token": " fixture-token-a ", "label": "Claude"}).Expect(http.StatusCreated)
	entry, _ := created.At("provider").(map[string]any)
	if entry["kind"] != "subscription" || entry["providerType"] != "claude-code" {
		t.Fatalf("the entry is %v", entry)
	}
	wantRefusal(t, b.Post("/api/providers/setup-token", master, backendtest.Map{"token": "fixture-token-c", "label": "Again"}),
		http.StatusConflict, "provider_exists", "a second subscription entry of the family")

	path := "/api/providers/" + entry["id"].(string) + "/accounts"
	second := b.Post(path, master, backendtest.Map{"token": "fixture-token-b"}).Expect(http.StatusCreated).Str("account.id")

	listed := b.Get(path, master).Expect(http.StatusOK)
	if strings.Contains(listed.Text(), "fixture-token") {
		t.Fatalf("the list returns a token: %s", listed.Body)
	}
	accounts, _ := listed.At("accounts").([]any)
	first, _ := listed.At("active").(string)
	if len(accounts) != 2 || first == "" || first == second {
		t.Fatalf("the accounts are %s", listed.Body)
	}
	// The data directory holds no token as it was given. That the secrets are
	// sealed in the data directory's files is the contract here.
	if dataHolds(t, h, "fixture-token") {
		t.Fatal("a token lies in the data directory as it was given")
	}

	// Selecting is explicit, and the active account cannot be removed.
	wantRefusal(t, b.Delete(path+"/"+first, master), http.StatusConflict, "active_account", "removing the active account")
	if active := b.Put(path+"/active", master, backendtest.Map{"credentialId": second}).Expect(http.StatusOK).Str("active"); active != second {
		t.Fatalf("the active account is %s", active)
	}
	b.Delete(path+"/"+first, master).Expect(http.StatusNoContent)
	wantRefusal(t, b.Put(path+"/active", master, backendtest.Map{"credentialId": "cred-missing"}), http.StatusNotFound, "account_not_found", "selecting a missing account")
	status := b.Get("/api/providers/"+entry["id"].(string)+"/status", master).Expect(http.StatusOK)
	if strings.Contains(status.Text(), "fixture-token") {
		t.Fatalf("the status returns a token: %s", status.Body)
	}
	if kept, _ := status.At("accounts").([]any); len(kept) != 1 || status.Str("active") != second {
		t.Fatalf("the status is %s", status.Body)
	}

	// Someone who only infers with a shared instance's entry sees whether it
	// works, not whose account it is.
	reader := b.CreateUser(master, "reader@example.test", "reader-pass-1", "user")
	wantRefusal(t, b.Post(path, reader, backendtest.Map{"token": "not-allowed"}), http.StatusForbidden, "forbidden", "a reader adding an account")
	hidden := b.Get(path, reader).Expect(http.StatusOK)
	if accounts, _ := hidden.At("accounts").([]any); len(accounts) != 0 || hidden.At("active") != nil {
		t.Fatalf("a reader sees %s", hidden.Body)
	}
	seen := b.Get("/api/providers/"+entry["id"].(string)+"/status", reader).Expect(http.StatusOK)
	if accounts, _ := seen.At("accounts").([]any); len(accounts) != 0 || seen.At("active") != nil || seen.At("quota") != nil {
		t.Fatalf("a reader sees %s", seen.Body)
	}
	backendtest.AssertJSON(t, seen.At("auth"), backendtest.Map{"status": "authenticated"})
	b.Stop()
}

// Cost: one backend and a scripted Codex, about a second.
func TestConcurrentDeviceLoginsPublishOneEntryAndTheOtherStoresNothing(t *testing.T) {
	t.Parallel()
	codex := scripted.StartCodex(t)
	// The catalog this scenario reads lists the vendors of a models.dev document
	// that names none.
	source := scripted.StartVendor(t)
	source.Handle(func(scripted.Request) *scripted.Response { return served(backendtest.Map{}) })
	h := backendtest.New(t, backendtest.WithCodex(codex), backendtest.WithModelsDev(source.URL("/api.json")))
	b, master := h.StartSetUp()
	for _, refusal := range []struct {
		body backendtest.Map
		code string
	}{
		{backendtest.Map{"providerType": "nope"}, "unknown_provider_type"},
		{backendtest.Map{"providerType": "anthropic"}, "no_login_flow"},
		{backendtest.Map{"type": "device"}, "invalid_body"},
	} {
		wantRefusal(t, b.Post(loginRoute, master, refusal.body), http.StatusBadRequest, refusal.code, string(backendtest.Marshal(refusal.body)))
	}
	first := startLogin(t, b, master, loginRoute, backendtest.Map{"providerType": "codex", "label": "Work"})
	second := startLogin(t, b, master, loginRoute, backendtest.Map{"providerType": "codex", "label": "Competing"})
	pending := untilLogin(t, b, master, first, func(state map[string]any) bool { return state["verificationUrl"] != nil })
	if pending["verificationUrl"] != codex.URL("/codex/device") || pending["userCode"] != scripted.CodexUserCode || pending["expiresAt"] == nil {
		t.Fatalf("the pending login is %v", pending)
	}

	codex.Approve(true)
	outcomes := []map[string]any{
		untilLogin(t, b, master, first, loginEnded),
		untilLogin(t, b, master, second, loginEnded),
	}
	var completed map[string]any
	failedThisScope := false
	for _, outcome := range outcomes {
		switch outcome["status"] {
		case "completed":
			if completed != nil {
				t.Fatalf("both logins completed: %v", outcomes)
			}
			completed = outcome
		case "failed":
			failedThisScope = failedThisScope || outcome["message"] == "This scope already has a codex subscription"
		}
	}
	if completed == nil || !failedThisScope {
		t.Fatalf("the outcomes are %v", outcomes)
	}
	providerID, credentialID := completed["providerId"].(string), completed["credentialId"].(string)
	entries := providerList(b, master)
	if len(entries) != 1 || backendtest.At(entries[0], "id") != providerID {
		t.Fatalf("the entries are %v", entries)
	}
	accounts := b.Get("/api/providers/"+providerID+"/accounts", master).Expect(http.StatusOK)
	if list, _ := accounts.At("accounts").([]any); len(list) != 1 || backendtest.At(list[0], "id") != credentialID || accounts.Str("active") != credentialID {
		t.Fatalf("the accounts are %s", accounts.Body)
	}
	if dataHolds(t, h, scripted.CodexRefreshToken) {
		t.Fatal("the login's refresh token lies in the data directory as it was given")
	}

	// One subscription entry per owner and family.
	subscriptions, _ := b.Get("/api/providers/catalog", master).Expect(http.StatusOK).At("subscriptions").([]any)
	offered := false
	for _, family := range subscriptions {
		offered = offered || (backendtest.At(family, "providerType") == "codex" && backendtest.At(family, "configured") == true)
	}
	if !offered {
		t.Fatalf("the catalog does not offer the configured family: %v", subscriptions)
	}
	wantRefusal(t, b.Post(loginRoute, master, backendtest.Map{"providerType": "codex"}), http.StatusConflict, "provider_exists", "a second login of the family")
	// A subscription entry takes a new label and nothing else.
	path := "/api/providers/" + providerID
	if label := b.Patch(path, master, backendtest.Map{"label": "Personal"}).Expect(http.StatusOK).Str("provider.label"); label != "Personal" {
		t.Fatalf("the label is %s", label)
	}
	wantRefusal(t, b.Patch(path, master, backendtest.Map{"apiKey": "k"}), http.StatusBadRequest, "subscription_only", "a key for a subscription entry")
	b.Delete(path, master).Expect(http.StatusNoContent)
	if entries := providerList(b, master); len(entries) != 0 {
		t.Fatalf("the deleted entry is listed: %v", entries)
	}
	b.Stop()
}

// Cost: one backend and a scripted Codex, about a second.
func TestALoginIntoAnEntryHoldsItUntilItEndsAndCancellingStopsItAtOnce(t *testing.T) {
	t.Parallel()
	codex := scripted.StartCodex(t)
	b, master := backendtest.New(t, backendtest.WithCodex(codex)).StartSetUp()
	entry := codexEntry(t, b, master, codex)
	codex.Approve(false)
	path := "/api/providers/" + entry["id"].(string)
	id := startLogin(t, b, master, path+"/accounts/login", backendtest.Map{})
	wantRefusal(t, b.Patch(path, master, backendtest.Map{"label": "Busy"}), http.StatusConflict, "provider_busy", "editing an entry a login holds")
	wantRefusal(t, b.Delete(path, master), http.StatusConflict, "provider_busy", "deleting an entry a login holds")
	backendtest.Eventually(t, "the login polls", func() bool { return codex.Polls() > 0 })
	b.Delete(loginRoute+"/"+id, master).Expect(http.StatusNoContent)
	backendtest.AssertJSON(t, loginState(b, master, id), backendtest.Map{"status": "failed", "message": "The login was cancelled"})
	b.Patch(path, master, backendtest.Map{"label": "Ready"}).Expect(http.StatusOK)
	// A cancelled login stops asking at once: no poll follows the ones in flight.
	settled := codex.Polls()
	time.Sleep(300 * time.Millisecond)
	if got := codex.Polls(); got > settled+1 {
		t.Fatalf("the cancelled login went on polling: %d polls after %d", got, settled)
	}
	wantRefusal(t, b.Delete(loginRoute+"/no-such-login", master), http.StatusNotFound, "login_not_found", "cancelling an unknown login")
	b.Stop()
}

// Cost: one backend and a scripted Codex, about 2.3 seconds. The runtime
// clock controls login expiry in Rust; a two-second lifetime leaves a second
// of scheduling allowance while still detecting a doubled lifetime.
func TestALoginExpiresAndItsResultGoesAfterTheRetention(t *testing.T) {
	t.Parallel()
	const lifetime, retention = 2 * time.Second, 300 * time.Millisecond
	codex := scripted.StartCodex(t)
	h := backendtest.New(t, backendtest.WithCodex(codex))
	h.Logins().LifetimeMs = backendtest.Ptr(uint64(lifetime.Milliseconds()))
	h.Logins().RetentionMs = backendtest.Ptr(uint64(retention.Milliseconds()))
	b, master := h.StartSetUp()
	started := time.Now()
	id := startLogin(t, b, master, loginRoute, backendtest.Map{"providerType": "codex"})
	expired := untilLogin(t, b, master, id, loginEnded)
	if elapsed := time.Since(started); elapsed > lifetime+time.Second {
		t.Fatalf("the login expired after %v, beyond its %v lifetime and scheduling allowance", elapsed, lifetime)
	}
	backendtest.AssertJSON(t, expired, backendtest.Map{"status": "failed", "message": "The login expired"})
	if entries := providerList(b, master); len(entries) != 0 {
		t.Fatalf("an expired login left an entry: %v", entries)
	}
	// The result stays for the retention after the login ended, and then goes: no
	// earlier than the lifetime and the retention after the start.
	backendtest.Eventually(t, "the login's result goes", func() bool {
		return b.Get(loginRoute+"/"+id, master).Status == http.StatusNotFound
	})
	if elapsed := time.Since(started); elapsed < lifetime+retention {
		t.Fatalf("the result went after %v", elapsed)
	}
	wantRefusal(t, b.Get(loginRoute+"/"+id, master), http.StatusNotFound, "login_not_found", "an expired login's result")
	b.Stop()
}

// Cost: one backend started twice and a scripted Codex, about a second.
func TestAFreeProbeFillsTheAccountsSnapshotWhichOutlivesARestart(t *testing.T) {
	t.Parallel()
	codex := scripted.StartCodex(t)
	h := backendtest.New(t, backendtest.WithCodex(codex))
	b, master := h.StartSetUp()
	entry := codexEntry(t, b, master, codex)
	path := "/api/providers/" + entry["id"].(string)
	status := b.Get(path+"/status", master).Expect(http.StatusOK)
	if status.At("quota") != nil {
		t.Fatalf("an account never probed has quota %v", status.At("quota"))
	}
	backendtest.AssertJSON(t, status.At("quotaCapability"), backendtest.Map{"type": "supported", "probe": "free"})

	// A body is optional: without one, the active account is probed.
	probed := b.Do(backendtest.Request{Method: http.MethodPost, Path: path + "/quota", Session: master}).Expect(http.StatusOK)
	snapshot, _ := probed.At("quota").(map[string]any)
	if snapshot["source"] != "probe" || snapshot["accountLabel"] != scripted.CodexAccountEmail || backendtest.At(snapshot, "windows.0.usedPercent") != 40.0 {
		t.Fatalf("the snapshot is %v", snapshot)
	}
	status = b.Get(path+"/status", master).Expect(http.StatusOK)
	backendtest.AssertJSON(t, status.At("quota"), snapshot)
	backendtest.AssertJSON(t, status.At("accounts.0.quota"), snapshot)
	wantRefusal(t, b.Post(path+"/quota", master, backendtest.Map{"credentialId": "cred-missing"}), http.StatusNotFound, "account_not_found", "probing a missing account")

	// The snapshot is the account's record, which a restart reads back.
	b.Stop()
	b = h.Start()
	restored := b.Get(path+"/status", master).Expect(http.StatusOK)
	backendtest.AssertJSON(t, restored.At("quota"), snapshot)
	b.Stop()
}

// Cost: one backend, about a second.
func TestAnAPIKeyEntryHasNoQuota(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	keyed := createProvider(b, master, backendtest.Map{"source": "custom", "providerType": "anthropic", "label": "Work", "apiKey": "k"})
	path := "/api/providers/" + keyed["id"].(string)
	if quota := b.Post(path+"/quota", master, backendtest.Map{}).Expect(http.StatusOK); quota.At("quota") != nil {
		t.Fatalf("a key entry has quota %s", quota.Body)
	}
	status := b.Get(path+"/status", master).Expect(http.StatusOK)
	backendtest.AssertJSON(t, status.At("quotaCapability"), backendtest.Map{"type": "none"})
	if accounts, _ := status.At("accounts").([]any); len(accounts) != 0 {
		t.Fatalf("a key entry has accounts %v", accounts)
	}
	b.Stop()
}
