package backend

import (
	"context"
	"slices"

	"golang.org/x/text/collate"
	"golang.org/x/text/language"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/anthropicapi"
	"github.com/wspl/demi/go/provider/openaiapi"
	"github.com/wspl/demi/go/webapi"
)

// vendorFamily is the family and wire a models.dev client package is
// written for.
type vendorFamily struct {
	npm, family string
	wire        *core.WireAPI
}

// vendorFamilies are the client packages a family speaks; a vendor of
// another package is not offered.
var vendorFamilies = []vendorFamily{
	{"@ai-sdk/openai-compatible", "openai", new(core.WireAPIChatCompletions)},
	{"@ai-sdk/openai", "openai", new(core.WireAPIResponses)},
	{"@ai-sdk/anthropic", "anthropic", nil},
	{"@ai-sdk/google", "google", nil},
}

// notOffered are the vendors whose endpoint needs an authentication scheme
// of its own, which an API key cannot satisfy.
var notOffered = []string{"github-copilot"}

// VendorCatalog is the vendor list over the backend's models.dev copy
// (providers.md § Vendors from models.dev): the vendors whose client package
// maps to a protocol one of the families speaks, each vendor's live model
// list, and the request requirements the backend applies to a vendor's
// models.
type VendorCatalog struct{ modelsDev *provider.ModelsDevClient }

// NewVendorCatalog lists the vendors of modelsDev.
func NewVendorCatalog(modelsDev *provider.ModelsDevClient) *VendorCatalog {
	return &VendorCatalog{modelsDev}
}

// ModelsDev is the backend's models.dev copy.
func (c *VendorCatalog) ModelsDev() *provider.ModelsDevClient { return c.modelsDev }

// Policy holds the request requirements of the models of vendor vendorID,
// which is nil for an entry added from no vendor.
func (c *VendorCatalog) Policy(vendorID *string) openaiapi.VendorPolicy {
	if vendorID != nil && *vendorID == "deepseek" {
		return openaiapi.VendorPolicy{PassBackReasoningContent: true}
	}
	return openaiapi.VendorPolicy{}
}

// Vendors are the vendors an entry can be added from, by name as the
// browser sorts names, from a copy less than a day old.
func (c *VendorCatalog) Vendors(ctx context.Context) ([]webapi.Vendor, error) {
	snapshot, err := c.modelsDev.Current(ctx)
	if err != nil {
		return nil, err
	}
	var vendors []webapi.Vendor
	for _, listed := range snapshot.Vendors() {
		if vendor, ok := offered(listed); ok {
			vendors = append(vendors, vendor)
		}
	}
	// The root locale's collation orders names as the browser's
	// localeCompare does. A collator is not safe for concurrent use.
	collator := collate.New(language.Und)
	slices.SortStableFunc(vendors, func(a, b webapi.Vendor) int { return collator.CompareString(a.Name, b.Name) })
	return vendors, nil
}

// Vendor is the vendor vendorID, when it is offered.
func (c *VendorCatalog) Vendor(ctx context.Context, vendorID string) (*webapi.Vendor, error) {
	snapshot, err := c.modelsDev.Current(ctx)
	if err != nil {
		return nil, err
	}
	listed := snapshot.Vendor(vendorID)
	if listed == nil {
		return nil, nil
	}
	vendor, ok := offered(*listed)
	if !ok {
		return nil, nil
	}
	return &vendor, nil
}

// Models is the live model list of vendor vendorID, read again: a vendor
// the document no longer offers has an empty list, never its family's.
func (c *VendorCatalog) Models(ctx context.Context, vendorID string) (core.ProviderModelList, error) {
	snapshot, err := c.modelsDev.Refreshed(ctx)
	if err != nil {
		return core.ProviderModelList{}, err
	}
	if listed := snapshot.Vendor(vendorID); listed != nil {
		if _, ok := offered(*listed); ok {
			if models := snapshot.VendorModels(vendorID); models != nil {
				return *models, nil
			}
		}
	}
	return core.ProviderModelList{Models: []core.ProviderModel{}, Warnings: slices.Clone(snapshot.Warnings), SourceFetchedAt: core.UnixEpoch, Stale: snapshot.Stale}, nil
}

// offered is the vendor as the page offers it, or false for one no family
// speaks.
func offered(vendor provider.ModelsDevVendor) (webapi.Vendor, bool) {
	if slices.Contains(notOffered, vendor.ID) || vendor.NPM == nil {
		return webapi.Vendor{}, false
	}
	index := slices.IndexFunc(vendorFamilies, func(family vendorFamily) bool { return family.npm == *vendor.NPM })
	if index < 0 {
		return webapi.Vendor{}, false
	}
	baseURL := vendor.API
	if baseURL == nil {
		baseURL = officialBaseURL(vendor.ID)
	}
	family := vendorFamilies[index]
	return webapi.Vendor{ID: vendor.ID, Name: vendor.Name, ProviderType: family.family, WireAPI: family.wire, BaseURL: baseURL, Doc: vendor.Doc}, true
}

// officialBaseURL is the endpoint of a first-party vendor that models.dev
// lists without one, because its own clients know it.
func officialBaseURL(vendorID string) *string {
	switch vendorID {
	case "anthropic":
		return new(anthropicapi.DefaultBaseURL)
	case "openai":
		return new(openaiapi.DefaultBaseURL)
	}
	return nil
}
