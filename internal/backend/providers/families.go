package providers

import (
	"context"
	"net/http"
	"net/url"
	"sort"
	"sync"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/providers/claudecode"
	"github.com/wspl/demi/internal/webapi"
)

// ProviderFamily builds the provider of one entry and account.
type ProviderFamily interface {
	// Credential says how the family entries authenticate.
	Credential() webapi.CredentialKind
	// Wires lists selectable protocols; empty means the family speaks one.
	Wires() []core.WireAPI
	// Provider builds one entry and account provider.
	Provider(FamilyArgs) (provider.Provider, error)
}

// ProcessFamily builds a session runtime whose process the placement starts.
// Families that run no process need not implement it.
type ProcessFamily interface {
	// ProcessRuntime builds a runtime over the supplied placement.
	ProcessRuntime(context.Context, FamilyArgs, claudecode.Placement) (provider.Runtime, error)
}

// FamilyArgs is what a family builds a provider from.
type FamilyArgs struct {
	EntryID    string
	Label      string
	Credential FamilyCredential
	HTTP       *http.Client
	Clock      core.Clock
	ModelsDev  *provider.ModelsDevClient
}

// FamilyCredential is the credential a provider stands for.
type FamilyCredential interface{ familyCredential() }

// APIKeyArgs is an API-key entry's settings, read only from the entry.
type APIKeyArgs struct {
	APIKey  provider.Secret
	BaseURL *url.URL
	WireAPI *core.WireAPI
	Vendor  provider.VendorPolicy
}

func (*APIKeyArgs) familyCredential() {}

// SubscriptionArgs is an entry's pool and the account its provider stands for.
type SubscriptionArgs struct {
	Pool    provider.CredentialPool
	Account *AccountBinding
}

func (*SubscriptionArgs) familyCredential() {}

// AccountBinding is the account a subscription provider stands for, with its quota snapshot.
type AccountBinding struct {
	CredentialID string
	Quota        provider.QuotaSnapshotStore
}

// FamilyRegistry is the families of a backend by name. Its zero value is ready to use.
type FamilyRegistry struct {
	// mu protects registry lookups and replacement; no family callback runs under it.
	mu       sync.Mutex
	families map[string]ProviderFamily
}

// Register replaces the family registered under name.
func (r *FamilyRegistry) Register(name string, family ProviderFamily) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.families == nil {
		r.families = make(map[string]ProviderFamily)
	}
	r.families[name] = family
}

// Family finds a registered family, or nil.
func (r *FamilyRegistry) Family(name string) ProviderFamily {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.families[name]
}

// Subscriptions returns the names of subscription families, in order.
func (r *FamilyRegistry) Subscriptions() []string {
	r.mu.Lock()
	snapshot := make(map[string]ProviderFamily, len(r.families))
	for name, family := range r.families {
		snapshot[name] = family
	}
	r.mu.Unlock()
	names := []string{}
	for name, family := range snapshot {
		if family.Credential() == webapi.CredentialKindSubscription {
			names = append(names, name)
		}
	}
	sort.Strings(names)
	return names
}
