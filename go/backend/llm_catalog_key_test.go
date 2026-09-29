package backend

import (
	"encoding/json/v2"
	"os"
	"testing"

	"github.com/wspl/demi/go/webapi"
)

// The key a catalog record is kept under is the Rust's for the same entry,
// so a record the Rust backend stored serves the Go backend (storage.md §
// Encodings and digests).
// Cost: one small file.
func TestACatalogKeyIsTheRustsForTheSameEntry(t *testing.T) {
	var fixture struct {
		Provider               string `json:"provider"`
		Account                string `json:"account"`
		Config                 string `json:"config"`
		APIKeyCatalogKey       string `json:"apiKeyCatalogKey"`
		SubscriptionCatalogKey string `json:"subscriptionCatalogKey"`
	}
	data, err := os.ReadFile("testdata/rust-vault.json")
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	config, err := decode[APIKeyConfig]([]byte(fixture.Config))
	if err != nil {
		t.Fatal(err)
	}
	id, err := webapi.ParseProviderID(fixture.Provider)
	if err != nil {
		t.Fatal(err)
	}
	account, err := webapi.ParseCredentialID(fixture.Account)
	if err != nil {
		t.Fatal(err)
	}
	keys := map[string]struct {
		entry ProviderEntry
		rust  string
	}{
		"an API-key entry":     {ProviderEntry{ID: id, Family: "openai", Credential: config}, fixture.APIKeyCatalogKey},
		"a subscription entry": {ProviderEntry{ID: id, Family: "codex", Credential: Subscription{Active: &account}}, fixture.SubscriptionCatalogKey},
	}
	for name, key := range keys {
		if got := catalogKey(key.entry); got != key.rust {
			t.Errorf("%s: %s, the Rust's is %s", name, got, key.rust)
		}
	}
}
