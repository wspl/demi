package backend_test

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/go/backend"
	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/providertest"
	"github.com/wspl/demi/go/webapi"
)

// familyProvider is a provider of the built-in family over pool, for
// account. No test here makes a request.
func familyProvider(family string, pool provider.CredentialPool, account *backend.AccountBinding, clock core.Clock) provider.Provider {
	registered, ok := backend.BuiltinFamilies().Get(family)
	if !ok {
		panic("no family " + family)
	}
	args := backend.FamilyArgs{
		EntryID:    "entry-1",
		Label:      family,
		Credential: backend.SubscriptionArgs{Pool: pool, Account: account},
		HTTP:       &http.Client{},
		Clock:      clock,
		ModelsDev:  provider.NewModelsDevClient(&http.Client{}, provider.ModelsDevDefaultURL, clock),
	}
	return must(registered.Provider(args))
}

// A family built to log in offers the device login and stands for no
// account; built for an entry's account, it stands for that account and
// keeps its quota in the account's store (providers.md § Login and
// publication).
// Cost: in memory.
func TestTheSubscriptionFamiliesLogInByDeviceAndStandForTheirBoundAccount(t *testing.T) {
	if got := backend.BuiltinFamilies().Subscriptions(); !slices.Equal(got, []string{"claude-code", "codex", "grok-build"}) {
		t.Fatalf("subscription families %v", got)
	}
	now := must(core.ParseTimestamp("2026-09-18T14:00:00.000Z"))
	clock := providertest.FixedClock{Time: now}
	codex := fmt.Sprintf(`{"accessToken":%q,"refreshToken":"refresh-1","idToken":%q,"accountId":"acct-1","lastRefresh":%q}`,
		providertest.JWT([]byte(`{"exp":1900000000}`)), providertest.JWT([]byte(`{"email":"user@example.com"}`)), now.String())
	grok := `{"accessToken":"session-token","issuer":"https://auth.x.ai","clientId":"client-1","email":"user@example.com"}`
	for family, secret := range map[string]string{"codex": codex, "grok-build": grok} {
		name := map[string]string{"codex": "Codex", "grok-build": "Grok"}[family]
		// Built to log in: the device login, and no account yet.
		login := familyProvider(family, provider.NewMemoryCredentialPool(), nil, clock)
		if capability := login.(provider.AccountsProvider).Accounts().Capability(); capability != (provider.AccountsCapability{Login: true}) {
			t.Errorf("%s: capability %+v", family, capability)
		}
		state, ok := login.AuthStatus(t.Context()).(core.AuthStateUnauthenticated)
		if !ok || state.Message == nil || *state.Message != "No "+name+" account is signed in" {
			t.Errorf("%s: a login's provider is %#v", family, login.AuthStatus(t.Context()))
		}

		// Built for an entry's account: that account's secret and quota.
		pool := provider.NewMemoryCredentialPool()
		meta := provider.AccountMeta{ID: "cred-1", Label: "user@example.com", UpdatedAt: now, Source: "login:device"}
		if err := pool.Write(t.Context(), meta, secret); err != nil {
			t.Fatal(err)
		}
		quota := &provider.MemorySnapshots{}
		label := "user@example.com"
		quota.Update(func(*core.QuotaSnapshot) core.QuotaSnapshot {
			return core.QuotaSnapshot{ObservedAt: now, Source: core.SnapshotSourceProbe, AccountLabel: &label, Windows: []core.QuotaWindow{}}
		})
		made := familyProvider(family, pool, &backend.AccountBinding{CredentialID: "cred-1", Quota: quota}, clock)
		signedIn, ok := made.AuthStatus(t.Context()).(core.AuthStateAuthenticated)
		if !ok || signedIn.AccountLabel == nil || *signedIn.AccountLabel != label {
			t.Errorf("%s: an account's provider is %#v", family, made.AuthStatus(t.Context()))
		}
		if latest := made.(provider.QuotaProvider).Quota().Latest(); latest != quota.Latest() {
			t.Errorf("%s: the provider keeps its quota elsewhere: %+v", family, latest)
		}
	}
}

// configuredEntry is an openai entry whose configured list holds models.
func configuredEntry(models ...webapi.ConfiguredModel) backend.ProviderEntry {
	config := backend.APIKeyConfig{APIKey: must(provider.NewSecret("sk-1"))}
	if models != nil {
		config.Models = &webapi.ConfiguredModels{Models: models}
	}
	return backend.ProviderEntry{ID: must(webapi.ParseProviderID("entry-1")), Family: "openai", Label: "Work", Credential: config}
}

func configured(outputLimit uint32) webapi.ConfiguredModel {
	extensions := []core.FileExtension{core.FileExtensionPng, core.FileExtensionPdf}
	return webapi.ConfiguredModel{ID: "gpt-5.5", DisplayName: "GPT-5.5", ContextWindow: 272000, OutputLimit: &outputLimit,
		ThinkingEfforts: []string{"low", "high"}, AcceptedExtensions: &extensions, FastTier: new("priority")}
}

// Every request takes the facts of the entry's configured model again, so
// an edit of the list reaches the next request, and keeps the thinking and
// tier the user chose (models.md § Request parameters).
// Cost: in memory.
func TestEveryRequestTakesTheConfiguredFactsAndKeepsTheUsersChoices(t *testing.T) {
	var thinking core.ThinkingConfig = core.ThinkingConfigEffort{Effort: "high"}
	// The selection a browser sent with a conversation's first request, from
	// an older catalog.
	chosen := core.ModelSelection{
		ProviderID:    "entry-1",
		Model:         core.Model{ID: "gpt-5.5", Name: "stale", ContextWindow: 1000, OutputLimit: new(uint32(100)), AcceptedExtensions: &[]core.FileExtension{}},
		Thinking:      &thinking,
		ServiceTierID: new("priority"),
	}
	applied := must(backend.ConfiguredSelection(configuredEntry(configured(4000)), chosen))
	if applied.Model.Name != "GPT-5.5" || applied.Model.ContextWindow != 272000 || *applied.Model.OutputLimit != 4000 {
		t.Fatalf("applied %+v", applied.Model)
	}
	if extensions := *applied.Model.AcceptedExtensions; !slices.Equal(extensions, []core.FileExtension{core.FileExtensionPng, core.FileExtensionPdf}) {
		t.Fatalf("extensions %v", extensions)
	}
	if applied.Thinking != chosen.Thinking || applied.ServiceTierID != chosen.ServiceTierID {
		t.Fatalf("the user's choices changed: %+v", applied)
	}
	// An edit of the list reaches the next request.
	if edited := must(backend.ConfiguredSelection(configuredEntry(configured(8000)), chosen)); *edited.Model.OutputLimit != 8000 {
		t.Fatalf("edited %+v", edited.Model)
	}
	// A model the list does not name fails the request; an entry without a
	// list keeps the selection as it is.
	other := chosen
	other.Model.ID = "gpt-4"
	var missing *backend.NotConfiguredError
	if _, err := backend.ConfiguredSelection(configuredEntry(configured(8000)), other); !errors.As(err, &missing) || missing.Model != "gpt-4" {
		t.Fatalf("an unlisted model: %v", err)
	}
	if kept := must(backend.ConfiguredSelection(configuredEntry(), chosen)); !reflect.DeepEqual(kept, chosen) {
		t.Fatalf("kept %+v", kept)
	}
}

// catalogNamed is a one-model catalog whose model is called name.
func catalogNamed(name string) core.ProviderModelList {
	model := core.ProviderModel{ID: "model", DisplayName: name, ContextWindow: new(uint32(1000)), SupportsTools: new(true),
		SupportsAttachments: new(false), SupportsReasoning: new(false), ServiceTiers: []core.ServiceTier{}}
	return core.ProviderModelList{Models: []core.ProviderModel{model}, Warnings: []string{}, SourceFetchedAt: must(core.ParseTimestamp("2026-09-13T00:00:00.000Z"))}
}

func name(list core.ProviderModelList) string { return list.Models[0].DisplayName }

// openCatalogs opens a control database holding one entry whose catalog a
// cache keeps.
func openCatalogs(t *testing.T) (*storage.Control, webapi.ProviderID) {
	t.Helper()
	control := must(storage.OpenControl(t.Context(), filepath.Join(t.TempDir(), "control.sqlite"), core.SystemClock{}))
	t.Cleanup(func() { control.Close() })
	hash := must(storage.ParsePasswordHash("$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0$0mUbQTTMhhaEBFGMq7WTZxOlVoS9sY3qVqLiV7Q1Izo"))
	master := must(control.CreateMaster(t.Context(), must(webapi.ParseEmailAddress("master@example.test")), hash))
	id := must(webapi.ParseProviderID("provider"))
	entry := storage.NewProvider{ID: id, Owner: master.ID, Family: "scripted", Kind: webapi.CredentialKindAPIKey, Label: "Scripted", Config: []byte("sealed")}
	must(control.InsertProvider(t.Context(), entry, nil))
	return control, id
}

// answering is a source that answers list, or err, and counts its reads.
func answering(reads *atomic.Int32, list core.ProviderModelList, err error) backend.CatalogFetch {
	return func(context.Context) (core.ProviderModelList, error) {
		reads.Add(1)
		return list, err
	}
}

// awaited is a source that answers what the test sends, says when it
// started, and says when it gave up before answering.
type awaited struct {
	answer           chan core.ProviderModelList
	started, dropped chan struct{}
}

func newAwaited() awaited {
	return awaited{answer: make(chan core.ProviderModelList), started: make(chan struct{}), dropped: make(chan struct{})}
}

func (a awaited) fetch(reads *atomic.Int32) backend.CatalogFetch {
	return func(ctx context.Context) (core.ProviderModelList, error) {
		reads.Add(1)
		close(a.started)
		select {
		case list := <-a.answer:
			return list, nil
		case <-ctx.Done():
			close(a.dropped)
			return core.ProviderModelList{}, ctx.Err()
		}
	}
}

// A fresh record serves at once, from memory and after a restart from
// storage; an expired one serves at once, marked stale, while one refresh
// runs behind its readers, which a forced read joins (models.md § Catalog
// cache).
// Cost: one SQLite database, on fake time.
func TestAFreshRecordServesFromMemoryAndStorageAndAnExpiredOneRefreshesOnceBehindItsReaders(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		control, id := openCatalogs(t)
		var reads atomic.Int32
		cache := backend.NewModelCatalogCache(control, core.SystemClock{})
		cold := make([]core.ProviderModelList, 8)
		var readers sync.WaitGroup
		for i := range cold {
			readers.Go(func() {
				cold[i] = must(cache.Read(t.Context(), id, "key", answering(&reads, catalogNamed("First"), nil), false))
			})
		}
		readers.Wait()
		for _, list := range cold {
			if name(list) != "First" || list.Stale {
				t.Fatalf("a cold read answered %+v", list)
			}
		}
		must(cache.Read(t.Context(), id, "key", answering(&reads, catalogNamed("Other"), nil), false))
		if reads.Load() != 1 {
			t.Fatalf("the source was read %d times", reads.Load())
		}
		cache.Close()

		// A restart loads the stored record before it asks the source.
		cache = backend.NewModelCatalogCache(control, core.SystemClock{})
		defer cache.Close()
		if restored := must(cache.Read(t.Context(), id, "key", answering(&reads, catalogNamed("Other"), nil), false)); name(restored) != "First" || reads.Load() != 1 {
			t.Fatalf("a restarted cache answered %q after %d reads", name(restored), reads.Load())
		}

		// Expired: every reader gets the record at once, marked stale, and
		// one refresh starts; a forced read joins it.
		time.Sleep(15 * time.Minute)
		source := newAwaited()
		for i := range 8 {
			fetch := answering(&reads, catalogNamed("Never"), nil)
			if i == 0 {
				fetch = source.fetch(&reads)
			}
			if list := must(cache.Read(t.Context(), id, "key", fetch, false)); !list.Stale || name(list) != "First" {
				t.Fatalf("an expired read answered %+v", list)
			}
		}
		<-source.started
		if reads.Load() != 2 {
			t.Fatalf("the source was read %d times", reads.Load())
		}
		var forced core.ProviderModelList
		readers.Go(func() {
			forced = must(cache.Read(t.Context(), id, "key", answering(&reads, catalogNamed("Never"), nil), true))
		})
		synctest.Wait()
		source.answer <- catalogNamed("Updated")
		readers.Wait()
		if name(forced) != "Updated" || forced.Stale || reads.Load() != 2 {
			t.Fatalf("the forced read answered %+v after %d reads", forced, reads.Load())
		}
		stored := must(control.CatalogRecord(t.Context(), id))
		if stored.CheckedAt != (core.SystemClock{}).Now() || name(stored.Catalog) != "Updated" {
			t.Fatalf("stored %+v", stored)
		}
	})
}

// A failed refresh answers the kept record, marked stale with the failure,
// and holds off automatic refreshes for a minute, during which readers get
// the record so marked however fresh it is; a forced refresh does not wait,
// and a later success replaces the record.
// Cost: one SQLite database, on fake time.
func TestAFailedRefreshKeepsTheRecordHoldsOffAMinuteAndAForcedRefreshDoesNotWait(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		control, id := openCatalogs(t)
		var reads, failures atomic.Int32
		cache := backend.NewModelCatalogCache(control, core.SystemClock{})
		defer cache.Close()
		offline := errors.New("offline")
		must(cache.Read(t.Context(), id, "key", answering(&reads, catalogNamed("First"), nil), false))
		kept := must(control.CatalogRecord(t.Context(), id))
		failed := must(cache.Read(t.Context(), id, "key", answering(&failures, core.ProviderModelList{}, offline), true))
		if !failed.Stale || name(failed) != "First" || !slices.Equal(failed.Warnings, []string{"offline"}) {
			t.Fatalf("a failed refresh answered %+v", failed)
		}
		if stored := must(control.CatalogRecord(t.Context(), id)); !reflect.DeepEqual(stored, kept) {
			t.Fatalf("a failed refresh stored %+v", stored)
		}
		// Held off: no automatic refresh for a minute, and the fresh record
		// goes out marked with the failure.
		held := must(cache.Read(t.Context(), id, "key", answering(&failures, core.ProviderModelList{}, offline), false))
		if !held.Stale || !slices.Equal(held.Warnings, []string{"offline"}) || failures.Load() != 1 {
			t.Fatalf("held off: %+v after %d failures", held, failures.Load())
		}
		time.Sleep(59 * time.Second)
		must(cache.Read(t.Context(), id, "key", answering(&failures, core.ProviderModelList{}, offline), true))
		if failures.Load() != 2 {
			t.Fatalf("a forced refresh waited: %d failures", failures.Load())
		}
		recovered := must(cache.Read(t.Context(), id, "key", answering(&reads, catalogNamed("Recovered"), nil), true))
		if name(recovered) != "Recovered" || recovered.Stale || len(recovered.Warnings) != 0 {
			t.Fatalf("recovered %+v", recovered)
		}
	})
}

// Invalidating an entry, or reading it under another key, cancels its
// refresh, whose answer then writes nothing; the new key starts afresh and
// never serves the old key's record.
// Cost: one SQLite database, on fake time.
func TestAChangedKeyOrAnInvalidationCancelsTheRefreshAndItsAnswerWritesNothing(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		control, id := openCatalogs(t)
		var reads atomic.Int32
		cache := backend.NewModelCatalogCache(control, core.SystemClock{})
		defer cache.Close()
		source := newAwaited()
		old := make(chan error)
		go func() {
			_, err := cache.Read(t.Context(), id, "old-account", source.fetch(&reads), false)
			old <- err
		}()
		<-source.started
		before := time.Now()
		if err := cache.Invalidate(t.Context(), id); err != nil {
			t.Fatal(err)
		}
		<-source.dropped
		if err := <-old; err == nil || err.Error() != "The catalog refresh was cancelled" || time.Since(before) != 0 {
			t.Fatalf("the invalidated refresh ended after %v with %v", time.Since(before), err)
		}

		if fresh := must(cache.Read(t.Context(), id, "new-account", answering(&reads, catalogNamed("New account"), nil), false)); name(fresh) != "New account" {
			t.Fatalf("the new account's catalog is %q", name(fresh))
		}
		// A changed key starts afresh and never serves the other key's record.
		changed := must(cache.Read(t.Context(), id, "changed-config", answering(&reads, catalogNamed("Changed"), nil), false))
		if name(changed) != "Changed" || reads.Load() != 3 {
			t.Fatalf("the changed key's catalog is %q after %d reads", name(changed), reads.Load())
		}
		stored := must(control.CatalogRecord(t.Context(), id))
		if stored.Key != "changed-config" || name(stored.Catalog) != "Changed" {
			t.Fatalf("stored %+v", stored)
		}
		if err := control.DeleteProvider(t.Context(), id); err != nil {
			t.Fatal(err)
		}
		if left := must(control.CatalogRecord(t.Context(), id)); left != nil {
			t.Fatalf("a deleted entry's catalog is left: %+v", left)
		}
	})
}

// Without a record, a failure and an answer the cache cannot keep are the
// reader's error; a refresh is given up after ten seconds; and closing the
// cache ends the readers waiting on a refresh and every later read.
// Cost: one SQLite database, on fake time.
func TestColdFailuresAndUnusableAnswersAreExplicitARefreshTimesOutAndClosingEndsItsReaders(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		control, id := openCatalogs(t)
		var reads atomic.Int32
		cache := backend.NewModelCatalogCache(control, core.SystemClock{})
		refused := func(list core.ProviderModelList, err error) string {
			_, err = cache.Read(t.Context(), id, "key", answering(&reads, list, err), true)
			if err == nil {
				t.Fatal("the read answered")
			}
			return err.Error()
		}
		if message := refused(core.ProviderModelList{}, errors.New("offline")); message != "offline" {
			t.Fatalf("offline: %s", message)
		}
		invalid := catalogNamed("Invalid")
		invalid.Models[0].OutputLimit = new(uint32(0))
		if message := refused(invalid, nil); !strings.Contains(message, "cannot be read") {
			t.Fatalf("invalid: %s", message)
		}
		stale := catalogNamed("Stale")
		stale.Stale = true
		stale.Warnings = []string{"Using stale models.dev catalog: offline"}
		if message := refused(stale, nil); message != "Using stale models.dev catalog: offline" {
			t.Fatalf("stale: %s", message)
		}
		if stored := must(control.CatalogRecord(t.Context(), id)); stored != nil {
			t.Fatalf("stored %+v", stored)
		}

		// A source that never answers is given up after ten seconds.
		never := newAwaited()
		before := time.Now()
		_, err := cache.Read(t.Context(), id, "key", never.fetch(&reads), true)
		if err == nil || err.Error() != "Model catalog request timed out" || time.Since(before) != 10*time.Second {
			t.Fatalf("%v after %v", err, time.Since(before))
		}
		<-never.dropped

		// Closing cancels a refresh and ends the readers waiting on it.
		pending := newAwaited()
		ended := make(chan error)
		go func() {
			_, err := cache.Read(t.Context(), id, "key", pending.fetch(&reads), true)
			ended <- err
		}()
		<-pending.started
		cache.Close()
		if err := <-ended; err == nil {
			t.Fatal("a reader of a cancelled refresh was answered")
		}
		if _, err := cache.Read(t.Context(), id, "key", answering(&reads, catalogNamed("Late"), nil), false); err == nil || err.Error() != "The model catalog cache is closed" {
			t.Fatalf("a closed cache read: %v", err)
		}
	})
}

// The newest release is the version the distribution's pointer names, from
// that version's manifest; it is believed until a refresh, concurrent
// readers share one read, a manifest is read once, and what fails to read
// leaves what was read (claude-code.md § Which version).
// Cost: a local HTTP server.
func TestTheNewestReleaseIsThePointersVersionFromItsManifestBelievedUntilARefresh(t *testing.T) {
	vendor := providertest.NewMockVendor(t)
	text := func(body string) providertest.MockResponse {
		return providertest.MockResponse{Status: 200, Chunks: []string{body}}
	}
	manifest := func(version string) providertest.MockResponse {
		return text(fmt.Sprintf(`{"version":%q,"commit":"ignored","platforms":{"linux-x64":{"binary":"claude","checksum":%q,"size":1024},"win32-x64":{"binary":"claude.exe","checksum":%q,"size":2048}}}`,
			version, strings.Repeat("ab", 32), strings.Repeat("cd", 32)))
	}
	releases := backend.NewClaudeReleases(*must(url.Parse(vendor.Server.URL + "/r")))
	vendor.Route("/r/latest", text("2.1.3\n"))
	vendor.Route("/r/2.1.3/manifest.json", manifest("2.1.3"))
	// Concurrent readers share one read.
	read := make([]error, 2)
	var readers sync.WaitGroup
	for i := range read {
		readers.Go(func() { _, read[i] = releases.Latest(t.Context(), false) })
	}
	readers.Wait()
	release := must(releases.Latest(t.Context(), false))
	if err := errors.Join(read...); err != nil || release.Version != "2.1.3" {
		t.Fatalf("read %q: %v", release.Version, err)
	}
	linux := release.Platforms["linux-x64"]
	if linux.URL != vendor.Server.URL+"/r/2.1.3/linux-x64/claude" || linux.Size != 1024 || linux.SHA256 != strings.Repeat("ab", 32) {
		t.Fatalf("linux %+v", linux)
	}
	if windows := release.Platforms["win32-x64"]; windows.URL != vendor.Server.URL+"/r/2.1.3/win32-x64/claude.exe" {
		t.Fatalf("windows %+v", windows)
	}
	if requests := len(vendor.Requests()); requests != 2 {
		t.Fatalf("%d requests", requests)
	}

	// A refresh reads the pointer again; a version's manifest is read once.
	vendor.Route("/r/latest", text("2.1.3"))
	if again := must(releases.Latest(t.Context(), true)); !reflect.DeepEqual(again, release) || len(vendor.Requests()) != 3 {
		t.Fatalf("a refresh read %+v in %d requests", again, len(vendor.Requests()))
	}

	// A pointer that names no version, a release that cannot be read and a
	// redirect fail; no other version is chosen, and what was read stays.
	failed := func(responses map[string]providertest.MockResponse) string {
		for path, response := range responses {
			vendor.Route(path, response)
		}
		_, err := releases.Latest(t.Context(), true)
		if err == nil {
			t.Fatal("the read answered")
		}
		return err.Error()
	}
	if message := failed(map[string]providertest.MockResponse{"/r/latest": text("latest-and-greatest")}); message != "the distribution named no version" {
		t.Fatalf("unnamed: %s", message)
	}
	missing := failed(map[string]providertest.MockResponse{"/r/latest": text("2.1.4"), "/r/2.1.4/manifest.json": {Status: 404}})
	if missing != "the release of version 2.1.4 could not be read (the distribution answered 404 Not Found)" {
		t.Fatalf("missing: %s", missing)
	}
	mislabelled := failed(map[string]providertest.MockResponse{"/r/latest": text("2.1.5"), "/r/2.1.5/manifest.json": manifest("2.1.4")})
	if !strings.HasSuffix(mislabelled, "(its manifest names version 2.1.4)") {
		t.Fatalf("mislabelled: %s", mislabelled)
	}
	moved := providertest.MockResponse{Status: 302, Headers: http.Header{"Location": {vendor.Server.URL + "/r/moved"}}}
	if redirected := failed(map[string]providertest.MockResponse{"/r/latest": moved}); redirected != "the distribution answered 302 Found" {
		t.Fatalf("redirected: %s", redirected)
	}
	if kept := must(releases.Latest(t.Context(), false)); !reflect.DeepEqual(kept, release) {
		t.Fatalf("kept %+v", kept)
	}
}
