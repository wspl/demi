package backend

import (
	"context"
	"net/http"
	"net/url"
	"reflect"
	"sync"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/webapi"
)

// UnknownFamilyError is an entry or a request naming a family this backend
// does not register.
type UnknownFamilyError struct{ Family string }

func (e *UnknownFamilyError) Error() string {
	return "the provider family " + e.Family + " is not available"
}

// ProviderAssembly builds the provider of each entry and account
// (providers.md § Inference admission and runtime ownership): through the
// entry's family from a fresh read of the entry, reused only while the entry
// read is unchanged, so a lookup that completes after an edit cannot keep
// later requests on a stale configuration. The edge and the shards share
// it.
type ProviderAssembly struct {
	vault    *Vault
	families FamilyRegistry
	quotas   *AccountQuotas
	catalogs *ModelCatalogCache
	vendors  *VendorCatalog
	// http is the shared services' client, which providers make their own
	// requests with.
	http  *http.Client
	clock core.Clock
	// mu guards built: lookups never wait under it.
	mu sync.Mutex
	// built is the provider of each entry's active account, with the entry
	// read it was built from.
	built map[webapi.ProviderID]builtProvider
}

type builtProvider struct {
	from     ProviderEntry
	provider provider.Provider
}

// NewProviderAssembly assembles the providers of vault's entries through
// families.
func NewProviderAssembly(vault *Vault, families FamilyRegistry, quotas *AccountQuotas, catalogs *ModelCatalogCache, vendors *VendorCatalog, http *http.Client, clock core.Clock) *ProviderAssembly {
	return &ProviderAssembly{vault: vault, families: families, quotas: quotas, catalogs: catalogs, vendors: vendors, http: http, clock: clock, built: map[webapi.ProviderID]builtProvider{}}
}

// Families are the families entries are built with.
func (a *ProviderAssembly) Families() FamilyRegistry { return a.families }

// Quotas are the accounts' quota snapshots.
func (a *ProviderAssembly) Quotas() *AccountQuotas { return a.quotas }

// Vendors is the vendor list.
func (a *ProviderAssembly) Vendors() *VendorCatalog { return a.vendors }

// Family is the family name, or an *UnknownFamilyError.
func (a *ProviderAssembly) Family(name string) (ProviderFamily, error) {
	family, ok := a.families.Get(name)
	if !ok {
		return nil, &UnknownFamilyError{name}
	}
	return family, nil
}

// ProviderFor is the provider of entry, a fresh read of the entry, for its
// active account: the one built before while the entry is unchanged,
// otherwise a new one.
func (a *ProviderAssembly) ProviderFor(ctx context.Context, entry ProviderEntry) (provider.Provider, error) {
	a.mu.Lock()
	reused, ok := a.built[entry.ID]
	a.mu.Unlock()
	// Entries are compared as the values they decode to.
	if ok && reflect.DeepEqual(reused.from, entry) {
		return reused.provider, nil
	}
	made, err := a.build(ctx, entry, entry.Active())
	if err != nil {
		return nil, err
	}
	// Two builds for one entry race only to store the same provider.
	a.mu.Lock()
	a.built[entry.ID] = builtProvider{entry, made}
	a.mu.Unlock()
	return made, nil
}

// ForAccount is the provider of entry for its account account, whichever
// is active; nil when the entry holds no such account.
func (a *ProviderAssembly) ForAccount(ctx context.Context, entry ProviderEntry, account webapi.CredentialID) (provider.Provider, error) {
	if active := entry.Active(); active != nil && *active == account {
		return a.ProviderFor(ctx, entry)
	}
	stored, err := a.vault.Account(ctx, entry.ID, account)
	if err != nil || stored == nil {
		return nil, err
	}
	return a.build(ctx, entry, &account)
}

// Detached is a provider of family over pool with no account yet, for a
// login whose entry does not exist until the login completes.
func (a *ProviderAssembly) Detached(family, id, label string, pool provider.CredentialPool) (provider.Provider, error) {
	registered, err := a.Family(family)
	if err != nil {
		return nil, err
	}
	return registered.Provider(a.args(id, label, SubscriptionArgs{Pool: pool}))
}

// RunsAProcess says whether entry's provider runs a process on a Host,
// which then is the user's Cloud (claude-code.md § Where it runs).
func (a *ProviderAssembly) RunsAProcess(ctx context.Context, entry ProviderEntry) (bool, error) {
	made, err := a.ProviderFor(ctx, entry)
	if err != nil {
		return false, err
	}
	return made.Capabilities().ProcessHost, nil
}

// Invalidate forgets the entry's provider and catalog after its
// configuration or active account changed.
func (a *ProviderAssembly) Invalidate(ctx context.Context, id webapi.ProviderID) error {
	a.mu.Lock()
	delete(a.built, id)
	a.mu.Unlock()
	return a.catalogs.Invalidate(ctx, id)
}

// Forget forgets everything held of a deleted entry.
func (a *ProviderAssembly) Forget(ctx context.Context, id webapi.ProviderID) error {
	a.quotas.ForgetEntry(id)
	return a.Invalidate(ctx, id)
}

// Close stops the catalog refreshes and waits for the quota writes.
func (a *ProviderAssembly) Close() {
	a.catalogs.Close()
	a.quotas.Close()
}

// build is entry's provider for account, built through its family.
func (a *ProviderAssembly) build(ctx context.Context, entry ProviderEntry, account *webapi.CredentialID) (provider.Provider, error) {
	family, err := a.Family(entry.Family)
	if err != nil {
		return nil, err
	}
	args, err := a.entryArgs(ctx, entry, account)
	if err != nil {
		return nil, err
	}
	return family.Provider(args)
}

// entryArgs are what entry's family builds from for account: the entry's
// settings, or its pool with the account and its quota store.
func (a *ProviderAssembly) entryArgs(ctx context.Context, entry ProviderEntry, account *webapi.CredentialID) (FamilyArgs, error) {
	switch credential := entry.Credential.(type) {
	case APIKeyConfig:
		settings := APIKeyArgs{APIKey: credential.APIKey, WireAPI: credential.WireAPI, Vendor: a.vendors.Policy(credential.VendorID)}
		if credential.BaseURL != nil {
			base, err := url.Parse(credential.BaseURL.String())
			if err != nil {
				return FamilyArgs{}, err
			}
			settings.BaseURL = base
		}
		return a.args(entry.ID.String(), entry.Label, settings), nil
	default:
		subscription := SubscriptionArgs{Pool: a.vault.Pool(entry.ID)}
		if account != nil {
			record, err := a.vault.Account(ctx, entry.ID, *account)
			if err != nil {
				return FamilyArgs{}, err
			}
			if record != nil {
				subscription.Account = &AccountBinding{CredentialID: record.ID.String(), Quota: a.quotas.Store(entry.ID, *record)}
			}
		}
		return a.args(entry.ID.String(), entry.Label, subscription), nil
	}
}

func (a *ProviderAssembly) args(entryID, label string, credential FamilyCredential) FamilyArgs {
	return FamilyArgs{EntryID: entryID, Label: label, Credential: credential, HTTP: a.http, Clock: a.clock, ModelsDev: a.vendors.ModelsDev()}
}
