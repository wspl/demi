//revive:disable:unused-parameter // API checkpoint keeps parameter names for callers; bodies follow after merge.
package providers

import (
	"context"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/webapi"
)

// CatalogFetch reads a catalog source when a refresh starts.
type CatalogFetch func(context.Context) (core.ProviderModelList, error)

// ModelCatalogCache holds validated catalogs in memory and the control store.
// Close cancels and joins refreshes independently owned from their readers.
type ModelCatalogCache struct{}

// VendorCatalog supplies supported models.dev vendors and their policies.
type VendorCatalog struct{}

// NewModelCatalogCache creates a cache whose refreshes are owned until Close.
func NewModelCatalogCache(control *database.ControlService, clock core.Clock) *ModelCatalogCache {
	panic("not written: b-providers")
}

// Read returns a fresh record, a stale record while refreshing, or waits when cold or forced.
func (c *ModelCatalogCache) Read(ctx context.Context, id webapi.ProviderID, key string, fetch CatalogFetch, force bool) (core.ProviderModelList, error) {
	panic("not written: b-providers")
}

// Invalidate cancels the entry refresh and removes its cached record.
func (c *ModelCatalogCache) Invalidate(ctx context.Context, id webapi.ProviderID) error {
	panic("not written: b-providers")
}

// Close cancels and joins all refreshes.
func (c *ModelCatalogCache) Close(ctx context.Context) error { panic("not written: b-providers") }

// NewVendorCatalog creates a catalog over the shared models.dev client.
func NewVendorCatalog(models *provider.ModelsDevClient) *VendorCatalog {
	panic("not written: b-providers")
}

// ModelsDev returns the shared models.dev client.
func (v *VendorCatalog) ModelsDev() *provider.ModelsDevClient { panic("not written: b-providers") }

// Policy returns the typed vendor request requirements.
func (v *VendorCatalog) Policy(id *string) provider.VendorPolicy { panic("not written: b-providers") }

// Vendors lists supported vendors.
func (v *VendorCatalog) Vendors(ctx context.Context) ([]webapi.Vendor, error) {
	panic("not written: b-providers")
}

// Vendor finds a supported vendor, or nil.
func (v *VendorCatalog) Vendor(ctx context.Context, id string) (*webapi.Vendor, error) {
	panic("not written: b-providers")
}

// Models reads a vendor catalog.
func (v *VendorCatalog) Models(ctx context.Context, id string) (core.ProviderModelList, error) {
	panic("not written: b-providers")
}

// ConfiguredModel converts configured facts to a catalog model.
func ConfiguredModel(model webapi.ConfiguredModel) core.ProviderModel {
	panic("not written: b-providers")
}

// ConfiguredSelection reapplies configured facts while keeping user thinking and tier choices.
func ConfiguredSelection(entry ProviderEntry, selection core.ModelSelection) (core.ModelSelection, error) {
	panic("not written: b-providers")
}

// Availability determines whether models can be used from provider health.
func Availability(auth core.AuthState, runtime core.RuntimeState) webapi.Availability {
	panic("not written: b-providers")
}
