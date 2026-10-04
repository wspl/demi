package providerhost

import (
	"context"
	"crypto/sha256"
	"fmt"
	"sort"

	"github.com/gowebpki/jcs"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/anthropicapi"
	"github.com/wspl/demi/internal/provider/openaiapi"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
	"golang.org/x/text/collate"
	"golang.org/x/text/language"
)

// CatalogFetch reads a catalog source when a refresh starts. It must honor context
// cancellation and release its IO before returning; the cache joins the call.
type CatalogFetch func(context.Context) (types.ProviderModelList, error)

// VendorCatalog supplies supported models.dev vendors and their policies.
type VendorCatalog struct{ models *provider.ModelsDevClient }

// NewVendorCatalog creates a catalog over the shared models.dev client.
func NewVendorCatalog(models *provider.ModelsDevClient) *VendorCatalog {
	return &VendorCatalog{models: models}
}

// ModelsDev returns the shared models.dev client.
func (v *VendorCatalog) ModelsDev() *provider.ModelsDevClient {
	return v.models
}

// Policy returns the typed vendor request requirements.
func (v *VendorCatalog) Policy(id *string) provider.VendorPolicy {
	if id == nil {
		return provider.VendorPolicy{}
	}
	return provider.VendorPolicy{
		PassBackReasoningContent: *id == "deepseek",
		EffortAsBudget:           *id != "anthropic",
	}
}

// Vendors lists supported vendors.
func (v *VendorCatalog) Vendors(ctx context.Context) ([]webapiproto.Vendor, error) {
	snapshot, err := v.models.Current(ctx)
	if err != nil {
		return nil, err
	}
	vendors := []webapiproto.Vendor{}
	for _, vendor := range snapshot.Vendors() {
		if offered := offeredVendor(vendor); offered != nil {
			vendors = append(vendors, *offered)
		}
	}
	collator := collate.New(language.Und)
	sort.SliceStable(
		vendors,
		func(i, j int) bool { return collator.CompareString(vendors[i].Name, vendors[j].Name) < 0 },
	)
	return vendors, nil
}

// Vendor finds a supported vendor, or nil.
func (v *VendorCatalog) Vendor(ctx context.Context, id string) (*webapiproto.Vendor, error) {
	snapshot, err := v.models.Current(ctx)
	if err != nil {
		return nil, err
	}
	vendor, ok := snapshot.Vendor(id)
	if !ok {
		return nil, nil
	}
	return offeredVendor(vendor), nil
}

// Models reads a vendor catalog.
func (v *VendorCatalog) Models(ctx context.Context, id string) (types.ProviderModelList, error) {
	snapshot, err := v.models.Refreshed(ctx)
	if err != nil {
		return types.ProviderModelList{}, err
	}
	if vendor, ok := snapshot.Vendor(id); ok && offeredVendor(vendor) != nil {
		if list, ok := snapshot.VendorModels(id); ok {
			return list, nil
		}
	}
	return types.ProviderModelList{
		Models:          []types.ProviderModel{},
		Warnings:        snapshot.Warnings,
		SourceFetchedAt: types.UnixEpoch,
		Stale:           snapshot.Stale,
	}, nil
}

// ConfiguredModel converts configured facts to a catalog model.
func ConfiguredModel(model webapiproto.ConfiguredModel) types.ProviderModel {
	efforts := make([]string, 0, len(model.ThinkingEfforts))
	for _, effort := range model.ThinkingEfforts {
		efforts = append(efforts, string(effort))
	}
	var first *string
	if len(efforts) > 0 {
		first = &efforts[0]
	}
	tools := true
	reasoning := len(efforts) > 0
	var attachments *bool
	if model.AcceptedExtensions != nil {
		v := len(*model.AcceptedExtensions) > 0
		attachments = &v
	}
	tiers := []types.ServiceTier{}
	if model.FastTier != nil {
		tiers = append(tiers, types.ServiceTier{ID: *model.FastTier, Label: "Fast", Fast: true})
	}
	return types.ProviderModel{
		ID:                       string(model.ID),
		DisplayName:              string(model.DisplayName),
		ContextWindow:            &model.ContextWindow,
		OutputLimit:              model.OutputLimit,
		SupportsTools:            &tools,
		SupportsAttachments:      attachments,
		AcceptedExtensions:       model.AcceptedExtensions,
		SupportsReasoning:        &reasoning,
		SupportedThinkingEfforts: &efforts,
		DefaultThinkingEffort:    first,
		ServiceTiers:             tiers,
	}
}

// ConfiguredSelection reapplies configured facts while keeping user thinking and tier choices.
func ConfiguredSelection(entry Entry, selection types.ModelSelection) (types.ModelSelection, error) {
	config, ok := entry.Credential.(*APIKeyConfig)
	if !ok || config.Models == nil {
		return selection, nil
	}
	for _, m := range *config.Models {
		if string(m.ID) == selection.Model.ID {
			return ConfiguredModel(m).Selection(string(entry.ID), selection.Thinking, selection.ServiceTierID), nil
		}
	}
	return types.ModelSelection{}, fmt.Errorf(
		"the model %s is not in the provider's configured list",
		selection.Model.ID,
	)
}

// Availability determines whether models can be used from provider health.
func Availability(auth types.AuthState, runtime types.RuntimeState) webapiproto.Availability {
	switch auth.(type) {
	case *types.Unauthenticated, *types.AuthError:
		return &webapiproto.AvailabilityUnavailable{
			Reason:  webapiproto.UnavailableReasonAuthentication,
			Message: "Provider login is unavailable",
		}
	case *types.AuthUnknown, *types.Authenticated:
	}
	switch r := runtime.(type) {
	case *types.RuntimeUnavailable:
		return &webapiproto.AvailabilityUnavailable{
			Reason:  webapiproto.UnavailableReasonRuntime,
			Message: r.Message,
		}
	case *types.RuntimeError:
		return &webapiproto.AvailabilityUnavailable{
			Reason:  webapiproto.UnavailableReasonRuntime,
			Message: r.Message,
		}
	case *types.RuntimeUnknown, *types.RuntimeReady:
	}
	return &webapiproto.AvailabilityAvailable{}
}

func offeredVendor(v provider.ModelsDevVendor) *webapiproto.Vendor {
	if v.ID == "github-copilot" || v.NPM == nil {
		return nil
	}
	var family string
	var wire *types.WireAPI
	switch *v.NPM {
	case "@ai-sdk/openai-compatible":
		family = "openai"
		value := types.WireAPIChatCompletions
		wire = &value
	case "@ai-sdk/openai":
		family = "openai"
		value := types.WireAPIResponses
		wire = &value
	case "@ai-sdk/anthropic":
		family = "anthropic"
	case "@ai-sdk/google":
		family = "google"
	default:
		return nil
	}
	base := v.API
	if base == nil {
		var value string
		switch v.ID {
		case "anthropic":
			value = anthropicapi.DefaultBaseURL
		case "openai":
			value = openaiapi.DefaultBaseURL
		}
		if value != "" {
			base = &value
		}
	}
	return &webapiproto.Vendor{
		ID:           v.ID,
		Name:         v.Name,
		ProviderType: family,
		WireAPI:      wire,
		BaseURL:      base,
		Doc:          v.Doc,
	}
}

// catalogKey is the hex SHA-256 of the RFC 8785 canonical JSON of the entry's
// family, API-key configuration and active account; stored catalogs are keyed by it.
func catalogKey(entry Entry) (string, error) {
	var config *APIKeyConfig
	if c, ok := entry.Credential.(*APIKeyConfig); ok {
		config = c
	}
	identity := struct {
		Family  string                    `json:"family"`
		Config  *APIKeyConfig             `json:"config"`
		Account *webapiproto.CredentialID `json:"account"`
	}{entry.Family, config, entry.Active()}
	data, err := contract.EncodeJSON(identity)
	if err != nil {
		return "", err
	}
	canonical, err := jcs.Transform(data)
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("%x", sha256.Sum256(canonical)), nil
}
