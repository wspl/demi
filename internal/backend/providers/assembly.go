//revive:disable:unused-parameter // API checkpoint keeps parameter names for callers; bodies follow after merge.
package providers

import (
	"context"
	"net/http"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/providers/claudecode"
	"github.com/wspl/demi/internal/webapi"
)

// Assembly is the providers of every entry, shared by the edge and shards.
type Assembly struct{}

// NewAssembly creates the shared provider assembly.
func NewAssembly(vault *Vault, families *FamilyRegistry, quotas *AccountQuotas, catalogs *ModelCatalogCache, vendors *VendorCatalog, http *http.Client, clock core.Clock) *Assembly {
	panic("not written: b-providers")
}

// Vault returns the assembly vault.
func (a *Assembly) Vault() *Vault { panic("not written: b-providers") }

// Families returns the assembly families.
func (a *Assembly) Families() *FamilyRegistry { panic("not written: b-providers") }

// Quotas returns the assembly quotas.
func (a *Assembly) Quotas() *AccountQuotas { panic("not written: b-providers") }

// Catalogs returns the assembly catalogs.
func (a *Assembly) Catalogs() *ModelCatalogCache { panic("not written: b-providers") }

// Vendors returns the assembly vendors.
func (a *Assembly) Vendors() *VendorCatalog { panic("not written: b-providers") }

// ProviderFor reuses a provider only while the fresh entry read is unchanged.
func (a *Assembly) ProviderFor(ctx context.Context, entry ProviderEntry) (provider.Provider, error) {
	panic("not written: b-providers")
}

// ForAccount builds the provider for an account, or nil when absent.
func (a *Assembly) ForAccount(ctx context.Context, entry ProviderEntry, account webapi.CredentialID) (provider.Provider, error) {
	panic("not written: b-providers")
}

// Detached builds a subscription provider before its entry exists.
func (a *Assembly) Detached(family, id, label string, pool provider.CredentialPool) (provider.Provider, error) {
	panic("not written: b-providers")
}

// RunsAProcess reports whether the entry requires a process on a Host.
func (a *Assembly) RunsAProcess(ctx context.Context, entry ProviderEntry) (bool, error) {
	panic("not written: b-providers")
}

// ProcessRuntime builds a session runtime over the supplied process placement.
func (a *Assembly) ProcessRuntime(ctx context.Context, entry ProviderEntry, account *webapi.CredentialID, placement claudecode.Placement) (provider.Runtime, error) {
	panic("not written: b-providers")
}

// Invalidate forgets the provider and catalog after a configuration or account change.
func (a *Assembly) Invalidate(ctx context.Context, id webapi.ProviderID) error {
	panic("not written: b-providers")
}

// Forget forgets everything held of a deleted entry.
func (a *Assembly) Forget(ctx context.Context, id webapi.ProviderID) error {
	panic("not written: b-providers")
}

// Close stops catalog refreshes and waits for quota writes.
func (a *Assembly) Close(ctx context.Context) error { panic("not written: b-providers") }

// Family returns the registered family or an unknown-family error.
func (a *Assembly) Family(name string) (ProviderFamily, error) { panic("not written: b-providers") }

// EntryCatalog reads the first available source; source failures become warnings.
func (a *Assembly) EntryCatalog(ctx context.Context, entry ProviderEntry, built provider.Provider, buildError error, refresh bool) core.ProviderModelList {
	panic("not written: b-providers")
}

// ModelCatalog returns each entry catalog and health independently.
func (a *Assembly) ModelCatalog(ctx context.Context, entries []ProviderEntry, refresh bool) []webapi.CatalogProvider {
	panic("not written: b-providers")
}

// Health returns authentication and runtime state; an accountless subscription is unauthenticated.
func (a *Assembly) Health(ctx context.Context, entry ProviderEntry, built provider.Provider, buildError error) (core.AuthState, core.RuntimeState) {
	panic("not written: b-providers")
}

// Details discloses accounts and quotas only to a configuring user.
func (a *Assembly) Details(ctx context.Context, entry ProviderEntry, disclose bool) (webapi.ProviderDetails, error) {
	panic("not written: b-providers")
}
