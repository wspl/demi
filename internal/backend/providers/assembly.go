package providers

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"reflect"
	"sync"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/providers/claudecode"
	"github.com/wspl/demi/internal/webapi"
)

// Assembly is the providers of every entry, shared by the edge and shards.
type Assembly struct {
	vault    *Vault
	families *FamilyRegistry
	quotas   *AccountQuotas
	catalogs *ModelCatalogCache
	vendors  *VendorCatalog
	http     *http.Client
	clock    core.Clock
	// mu protects provider reuse, never family callbacks or IO.
	mu    sync.Mutex
	built map[webapi.ProviderID]builtProvider
}
type builtProvider struct {
	entry    ProviderEntry
	provider provider.Provider
}

// NewAssembly creates the shared provider assembly.
func NewAssembly(
	vault *Vault,
	families *FamilyRegistry,
	quotas *AccountQuotas,
	catalogs *ModelCatalogCache,
	vendors *VendorCatalog,
	http *http.Client,
	clock core.Clock,
) *Assembly {
	return &Assembly{
		vault:    vault,
		families: families,
		quotas:   quotas,
		catalogs: catalogs,
		vendors:  vendors,
		http:     http,
		clock:    clock,
		built:    make(map[webapi.ProviderID]builtProvider),
	}
}

// Vault returns the assembly vault.
func (a *Assembly) Vault() *Vault {
	return a.vault
}

// Families returns the assembly families.
func (a *Assembly) Families() *FamilyRegistry {
	return a.families
}

// Quotas returns the assembly quotas.
func (a *Assembly) Quotas() *AccountQuotas {
	return a.quotas
}

// Catalogs returns the assembly catalogs.
func (a *Assembly) Catalogs() *ModelCatalogCache {
	return a.catalogs
}

// Vendors returns the assembly vendors.
func (a *Assembly) Vendors() *VendorCatalog {
	return a.vendors
}

// ProviderFor reuses a provider only while the fresh entry read is unchanged.
func (a *Assembly) ProviderFor(ctx context.Context, entry ProviderEntry) (provider.Provider, error) {
	a.mu.Lock()
	cached, ok := a.built[entry.ID]
	a.mu.Unlock()
	if ok && reflect.DeepEqual(cached.entry, entry) {
		return cached.provider, nil
	}
	p, err := a.build(ctx, entry, entry.Active())
	if err != nil {
		return nil, err
	}
	// Cache an owned entry snapshot so edits by a caller cannot alter the reuse key.
	snapshot, err := cloneEntry(entry)
	if err != nil {
		return nil, err
	}
	a.mu.Lock()
	a.built[entry.ID] = builtProvider{snapshot, p}
	a.mu.Unlock()
	return p, nil
}

// ForAccount builds the provider for an account, or nil when absent.
func (a *Assembly) ForAccount(
	ctx context.Context,
	entry ProviderEntry,
	account webapi.CredentialID,
) (provider.Provider, error) {
	if active := entry.Active(); active != nil && *active == account {
		return a.ProviderFor(ctx, entry)
	}
	record, err := a.vault.Account(ctx, entry.ID, account)
	if err != nil {
		return nil, &AssemblyError{Kind: AssemblyStorage, Err: err}
	}
	if record == nil {
		return nil, nil
	}
	return a.build(ctx, entry, &account)
}

// Detached builds a subscription provider before its entry exists.
func (a *Assembly) Detached(
	family, id, label string,
	pool provider.CredentialPool,
) (provider.Provider, error) {
	registered, err := a.Family(family)
	if err != nil {
		return nil, err
	}
	p, err := registered.Provider(a.args(id, label, &SubscriptionArgs{Pool: pool}))
	if err != nil {
		return nil, err
	}
	return p, nil
}

// RunsAProcess reports whether the entry requires a process on a Host.
func (a *Assembly) RunsAProcess(ctx context.Context, entry ProviderEntry) (bool, error) {
	p, err := a.ProviderFor(ctx, entry)
	if err != nil {
		return false, err
	}
	return p.Capabilities().ProcessHost, nil
}

// ProcessRuntime builds a session runtime over the supplied process placement.
func (a *Assembly) ProcessRuntime(
	ctx context.Context,
	entry ProviderEntry,
	account *webapi.CredentialID,
	placement claudecode.Placement,
) (provider.Runtime, error) {
	family, err := a.Family(entry.Family)
	if err != nil {
		return nil, err
	}
	process, ok := family.(ProcessFamily)
	if !ok {
		return nil, fmt.Errorf("the provider family %s cannot run its process", entry.Family)
	}
	args, err := a.entryArgs(ctx, entry, account)
	if err != nil {
		return nil, err
	}
	runtime, err := process.ProcessRuntime(ctx, args, placement)
	if err != nil {
		return nil, err
	}
	return runtime, nil
}

// Invalidate forgets the provider and catalog after a configuration or account change.
func (a *Assembly) Invalidate(ctx context.Context, id webapi.ProviderID) error {
	a.mu.Lock()
	delete(a.built, id)
	a.mu.Unlock()
	return a.catalogs.Invalidate(ctx, id)
}

// Forget forgets everything held of a deleted entry.
func (a *Assembly) Forget(ctx context.Context, id webapi.ProviderID) error {
	a.quotas.ForgetEntry(id)
	return a.Invalidate(ctx, id)
}

// Close stops catalog refreshes and waits for quota writes.
func (a *Assembly) Close(ctx context.Context) error {
	err := a.catalogs.Close(ctx)
	quotaErr := a.quotas.Close(ctx)
	return errors.Join(err, quotaErr)
}

// Family returns the registered family or an unknown-family error.
func (a *Assembly) Family(name string) (ProviderFamily, error) {
	family := a.families.Family(name)
	if family == nil {
		return nil, &AssemblyError{Kind: AssemblyUnknownFamily, Family: name}
	}
	return family, nil
}

// EntryCatalog reads the first available source; source failures become warnings.
func (a *Assembly) EntryCatalog(
	ctx context.Context,
	entry ProviderEntry,
	built provider.Provider,
	buildError error,
	refresh bool,
) core.ProviderModelList {
	if c, ok := entry.Credential.(*APIKeyConfig); ok && c.Models != nil {
		models := make([]core.ProviderModel, 0, len(*c.Models))
		for _, m := range *c.Models {
			models = append(models, ConfiguredModel(m))
		}
		return core.ProviderModelList{
			Models:          models,
			Warnings:        []string{},
			SourceFetchedAt: core.UnixEpoch,
		}
	}
	var fetch CatalogFetch
	if c, ok := entry.Credential.(*APIKeyConfig); ok && c.VendorID != nil {
		vendor := *c.VendorID
		fetch = func(ctx context.Context) (core.ProviderModelList, error) { return a.vendors.Models(ctx, vendor) }
	} else if buildError != nil {
		return emptyCatalog(buildError)
	} else {
		fetch = built.ListModels
	}
	key, err := catalogKey(entry)
	if err != nil {
		return emptyCatalog(err)
	}
	list, err := a.catalogs.Read(ctx, entry.ID, key, fetch, refresh)
	if err != nil {
		return emptyCatalog(err)
	}
	return list
}

// ModelCatalog returns each entry catalog and health independently.
func (a *Assembly) ModelCatalog(
	ctx context.Context,
	entries []ProviderEntry,
	refresh bool,
) []webapi.CatalogProvider {
	result := make([]webapi.CatalogProvider, len(entries))
	var workers sync.WaitGroup
	for i, entry := range entries {
		workers.Go(func() {
			p, err := a.ProviderFor(ctx, entry)
			list := a.EntryCatalog(ctx, entry, p, err, refresh)
			auth, runtime := a.Health(ctx, entry, p, err)
			models := make([]webapi.CatalogModel, 0, len(list.Models))
			for _, model := range list.Models {
				models = append(
					models,
					webapi.CatalogModel{
						ProviderModel: model,
						Selection:     model.Selection(string(entry.ID), nil, nil),
						UnnamedEffort: model.UnnamedEffort(),
					},
				)
			}
			var cli *string
			if p != nil {
				cli = CLIPackage(p)
			}
			result[i] = webapi.CatalogProvider{
				ProviderID:      entry.ID,
				DisplayName:     entry.Label,
				CLIPackage:      cli,
				Models:          models,
				SourceFetchedAt: list.SourceFetchedAt,
				Stale:           list.Stale,
				Warnings:        list.Warnings,
				Auth:            auth,
				Runtime:         runtime,
				Availability:    Availability(auth, runtime),
			}
		})
	}
	workers.Wait()
	return result
}

// Health returns authentication and runtime state; an accountless subscription is unauthenticated.
func (a *Assembly) Health(
	ctx context.Context,
	entry ProviderEntry,
	built provider.Provider,
	buildError error,
) (core.AuthState, core.RuntimeState) {
	if buildError != nil {
		return &core.AuthError{Message: buildError.Error()}, &core.RuntimeUnknown{}
	}
	if _, ok := entry.Credential.(*SubscriptionCredential); ok && entry.Active() == nil {
		message := "No subscription account configured"
		return &core.Unauthenticated{Message: &message}, built.RuntimeState()
	}
	return built.AuthStatus(ctx), built.RuntimeState()
}

// Details discloses accounts and quotas only to a configuring user.
func (a *Assembly) Details(
	ctx context.Context,
	entry ProviderEntry,
	disclose bool,
) (webapi.ProviderDetails, error) {
	p, err := a.ProviderFor(ctx, entry)
	auth, runtime := a.Health(ctx, entry, p, err)
	if err != nil {
		return webapi.ProviderDetails{}, err
	}
	accounts := []webapi.AccountDTO{}
	if _, ok := entry.Credential.(*SubscriptionCredential); ok {
		rows, err := a.vault.Accounts(ctx, entry.ID)
		if err != nil {
			return webapi.ProviderDetails{}, &AssemblyError{Kind: AssemblyStorage, Err: err}
		}
		for _, row := range rows {
			accounts = append(
				accounts,
				webapi.AccountDTO{
					AccountInfo: AccountMeta(row).Info(),
					Quota:       a.quotas.Latest(entry.ID, row),
				},
			)
		}
	}
	var quota *core.QuotaSnapshot
	active := entry.Active()
	for _, account := range accounts {
		if active != nil && account.ID == string(*active) {
			quota = account.Quota
		}
	}
	capability := quotaCapability(p)
	details := webapi.ProviderDetails{
		Auth:            auth,
		Runtime:         runtime,
		Accounts:        accounts,
		Active:          active,
		Quota:           quota,
		QuotaCapability: capability,
		CLIPackage:      CLIPackage(p),
	}
	if !disclose {
		details.Accounts = []webapi.AccountDTO{}
		details.Active = nil
		details.Quota = nil
		if _, ok := auth.(*core.Authenticated); ok {
			details.Auth = &core.Authenticated{}
		}
	}
	return details, nil
}

func (a *Assembly) args(id, label string, credential FamilyCredential) FamilyArgs {
	return FamilyArgs{
		EntryID:    id,
		Label:      label,
		Credential: credential,
		HTTP:       a.http,
		Clock:      a.clock,
		ModelsDev:  a.vendors.ModelsDev(),
	}
}

func (a *Assembly) entryArgs(
	ctx context.Context,
	entry ProviderEntry,
	account *webapi.CredentialID,
) (FamilyArgs, error) {
	var credential FamilyCredential
	switch c := entry.Credential.(type) {
	case *APIKeyConfig:
		var base *url.URL
		if c.BaseURL != nil {
			var err error
			base, err = url.Parse(string(*c.BaseURL))
			if err != nil {
				return FamilyArgs{}, errors.New("invalid provider endpoint")
			}
		}
		credential = &APIKeyArgs{
			APIKey:  c.APIKey,
			BaseURL: base,
			WireAPI: c.WireAPI,
			Vendor:  a.vendors.Policy(c.VendorID),
		}
	case *SubscriptionCredential:
		var binding *AccountBinding
		if account != nil {
			record, err := a.vault.Account(ctx, entry.ID, *account)
			if err != nil {
				return FamilyArgs{}, &AssemblyError{Kind: AssemblyStorage, Err: err}
			}
			if record != nil {
				binding = &AccountBinding{
					CredentialID: string(record.ID),
					Quota:        a.quotas.Store(entry.ID, *record),
				}
			}
		}
		credential = &SubscriptionArgs{Pool: a.vault.Pool(entry.ID), Account: binding}
	}
	return a.args(string(entry.ID), entry.Label, credential), nil
}

func (a *Assembly) build(
	ctx context.Context,
	entry ProviderEntry,
	account *webapi.CredentialID,
) (provider.Provider, error) {
	family, err := a.Family(entry.Family)
	if err != nil {
		return nil, err
	}
	args, err := a.entryArgs(ctx, entry, account)
	if err != nil {
		return nil, err
	}
	p, err := family.Provider(args)
	if err != nil {
		return nil, err
	}
	return p, nil
}

// cloneEntry freezes the configuration used as a provider reuse key.
func cloneEntry(entry ProviderEntry) (ProviderEntry, error) {
	switch c := entry.Credential.(type) {
	case *APIKeyConfig:
		b, err := contract.EncodeJSON(c)
		if err != nil {
			return ProviderEntry{}, err
		}
		value, err := DecodeAPIKeyConfig(b)
		if err != nil {
			return ProviderEntry{}, err
		}
		entry.Credential = &value
	case *SubscriptionCredential:
		next := &SubscriptionCredential{}
		if c.Active != nil {
			id := *c.Active
			next.Active = &id
		}
		entry.Credential = next
	}
	return entry, nil
}

func emptyCatalog(err error) core.ProviderModelList {
	return core.ProviderModelList{
		Models:          []core.ProviderModel{},
		Warnings:        []string{err.Error()},
		SourceFetchedAt: core.UnixEpoch,
	}
}

// quotaCapability presents the provider’s supported quota probe and its cost.
func quotaCapability(p provider.Provider) webapi.QuotaCapability {
	var capability webapi.QuotaCapability = &webapi.QuotaCapabilityNone{}
	if q := p.Quota(); q != nil {
		var cost *webapi.ProbeCost
		if c, ok := q.ProbeCost(); ok {
			value := webapi.ProbeCostFree
			if c == provider.ProbeInference {
				value = webapi.ProbeCostInference
			}
			cost = &value
		}
		capability = &webapi.QuotaCapabilitySupported{Probe: cost}
	}
	return capability
}
