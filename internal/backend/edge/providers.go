package edge

import (
	"errors"
	"fmt"
	"net/http"
	"slices"

	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/webapi"
)

func (e *Edge) configures(r *http.Request) error {
	if !e.state.Services.Vault.Configures(caller(r)) {
		return apiFailure(403, "forbidden", "Providers are configured by the instance owner")
	}
	return nil
}

func (e *Edge) scoped(r *http.Request) (*providers.ProviderEntry, error) {
	missing := apiFailure(404, "provider_not_found", "No such provider")
	id, err := webapi.ParseProviderID(r.PathValue("id"))
	if err != nil {
		return nil, missing
	}
	entry, err := e.state.Services.Vault.Visible(r.Context(), caller(r).ID, id)
	if err != nil {
		return nil, err
	}
	if entry == nil {
		return nil, missing
	}
	return entry, nil
}

func (e *Edge) reserve(entry *providers.ProviderEntry) (*providers.Reservation, error) {
	reservation := e.state.Services.Operations.Reserve(entry.ID)
	if reservation == nil {
		return nil, apiFailure(409, "provider_busy", "Another change of this provider is still running")
	}
	return reservation, nil
}

func (e *Edge) models(w http.ResponseWriter, r *http.Request) error {
	query, err := decodeQuery(r, webapi.DecodeRefresh, "refresh")
	if err != nil {
		return err
	}
	owner, err := e.state.Services.Vault.OwnerFor(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	entries, err := e.state.Services.Vault.Entries(r.Context(), owner)
	if err != nil {
		return err
	}
	writeJSON(
		w,
		200,
		webapi.ModelCatalog{
			Providers: e.state.Services.Assembly.ModelCatalog(r.Context(), entries, bool(query.Refresh)),
		},
	)
	return nil
}

func (e *Edge) providers(w http.ResponseWriter, r *http.Request) error {
	owner, err := e.state.Services.Vault.OwnerFor(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	entries, err := e.state.Services.Vault.Entries(r.Context(), owner)
	if err != nil {
		return err
	}
	result := make([]webapi.ProviderDTO, 0, len(entries))
	for _, entry := range entries {
		result = append(result, entry.DTO())
	}
	writeJSON(w, 200, webapi.Providers{Providers: result})
	return nil
}

func (e *Edge) vendorCatalog(w http.ResponseWriter, r *http.Request) error {
	owner, err := e.state.Services.Vault.OwnerFor(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	entries, err := e.state.Services.Vault.Entries(r.Context(), owner)
	if err != nil {
		return err
	}
	subscriptions := make([]webapi.SubscriptionFamily, 0)
	for _, family := range e.state.Services.Assembly.Families().Subscriptions() {
		configured := false
		for _, entry := range entries {
			if entry.Family == family {
				configured = true
			}
		}
		subscriptions = append(subscriptions, webapi.SubscriptionFamily{ProviderType: family, Configured: configured})
	}
	vendors, err := e.state.Services.Assembly.Vendors().Vendors(r.Context())
	if err != nil {
		return apiFailure(502, "catalog_unavailable", err.Error())
	}
	writeJSON(w, 200, webapi.VendorCatalog{Subscriptions: subscriptions, Vendors: vendors})
	return nil
}

func (e *Edge) apiKeyFamily(name string, wire *core.WireAPI) error {
	family := e.state.Services.Assembly.Families().Family(name)
	if family == nil {
		return apiFailure(400, "unknown_provider_type", fmt.Sprintf("Unknown provider type %q", name))
	}
	if family.Credential() == webapi.CredentialKindSubscription {
		return apiFailure(400, "subscription_only", fmt.Sprintf("Provider type %q is configured by its login", name))
	}
	if wire != nil && !slices.Contains(family.Wires(), *wire) {
		return apiFailure(400, "invalid_body", fmt.Sprintf("wireApi: the %s family does not speak %s", name, *wire))
	}
	return nil
}

func (e *Edge) createProvider(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeCreateProvider)
	if err != nil {
		return err
	}
	if err := e.configures(r); err != nil {
		return err
	}
	var family, label, key string
	var config providers.APIKeyConfig
	switch request := request.(type) {
	case *webapi.CreateProviderCustom:
		family = request.ProviderType
		label = string(request.Label)
		key = request.APIKey
		config = providers.APIKeyConfig{BaseURL: request.BaseURL, WireAPI: request.WireAPI, Models: request.Models}
	case *webapi.CreateProviderVendor:
		vendor, err := e.state.Services.Assembly.Vendors().Vendor(r.Context(), request.VendorID)
		if err != nil {
			return apiFailure(502, "catalog_unavailable", err.Error())
		}
		if vendor == nil {
			return apiFailure(400, "unknown_vendor", fmt.Sprintf("Unknown vendor %q", request.VendorID))
		}
		family = vendor.ProviderType
		label = string(request.Label)
		key = request.APIKey
		config = providers.APIKeyConfig{
			BaseURL:  request.BaseURL,
			WireAPI:  vendor.WireAPI,
			VendorID: &vendor.ID,
			Models:   request.Models,
		}
		if err := vendorEndpoint(&config, vendor, request.VendorID); err != nil {
			return err
		}
	}
	if err := e.apiKeyFamily(family, config.WireAPI); err != nil {
		return err
	}
	secret, err := provider.NewSecret(key)
	if err != nil {
		return apiFailure(400, "invalid_body", "apiKey: "+err.Error())
	}
	config.APIKey = secret
	owner, err := e.state.Services.Vault.OwnerFor(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	entry, err := e.state.Services.Vault.CreateAPIKey(r.Context(), owner, family, label, config)
	if err != nil {
		return err
	}
	writeJSON(w, 201, webapi.ProviderAnswer{Provider: entry.DTO()})
	return nil
}

func (e *Edge) patchProvider(w http.ResponseWriter, r *http.Request) error {
	patch, err := decodeBody(r, webapi.DecodeProviderPatch)
	if err != nil {
		return err
	}
	if err := e.configures(r); err != nil {
		return err
	}
	entry, err := e.scoped(r)
	if err != nil {
		return err
	}
	reservation, err := e.reserve(entry)
	if err != nil {
		return err
	}
	defer reservation.Release()
	config, err := patchedAPIKey(entry, patch)
	if err != nil {
		return err
	}
	var label *string
	if patch.Label != nil {
		value := string(*patch.Label)
		label = &value
	}
	updated, err := e.state.Services.Vault.Update(r.Context(), entry.ID, label, config)
	if err != nil {
		return err
	}
	if updated == nil {
		return apiFailure(404, "provider_not_found", "No such provider")
	}
	if config != nil {
		if err := e.state.Services.Assembly.Invalidate(r.Context(), entry.ID); err != nil {
			return err
		}
	}
	writeJSON(w, 200, webapi.ProviderAnswer{Provider: updated.DTO()})
	return nil
}

func (e *Edge) deleteProvider(w http.ResponseWriter, r *http.Request) error {
	if err := e.configures(r); err != nil {
		return err
	}
	entry, err := e.scoped(r)
	if err != nil {
		return err
	}
	reservation, err := e.reserve(entry)
	if err != nil {
		return err
	}
	defer reservation.Release()
	if err := e.state.Services.Vault.Delete(r.Context(), *entry); err != nil {
		return err
	}
	if err := e.state.Services.Assembly.Forget(r.Context(), entry.ID); err != nil {
		return err
	}
	e.state.Services.CLIInstalls.Forget(entry.ID)
	w.WriteHeader(204)
	return nil
}

func (e *Edge) providerStatus(w http.ResponseWriter, r *http.Request) error {
	entry, err := e.scoped(r)
	if err != nil {
		return err
	}
	result, err := e.state.Services.Assembly.Details(r.Context(), *entry, e.state.Services.Vault.Configures(caller(r)))
	if err != nil {
		var assembly *providers.AssemblyError
		if errors.As(err, &assembly) && assembly.Kind == providers.AssemblyStorage {
			return err
		}
		return apiFailure(502, "provider_status_failed", err.Error())
	}
	writeJSON(w, 200, result)
	return nil
}

func (e *Edge) accountProvider(
	r *http.Request,
	entry providers.ProviderEntry,
	account *webapi.CredentialID,
) (provider.Provider, error) {
	if account == nil {
		return e.state.Services.Assembly.ProviderFor(r.Context(), entry)
	}
	built, err := e.state.Services.Assembly.ForAccount(r.Context(), entry, *account)
	if err != nil {
		return nil, err
	}
	if built == nil {
		return nil, apiFailure(404, "account_not_found", "No such account")
	}
	return built, nil
}

func (e *Edge) quota(w http.ResponseWriter, r *http.Request) error {
	data, err := readJSONBody(r)
	if err != nil {
		return err
	}
	if len(data) == 0 {
		data = []byte("{}")
	}
	request, err := webapi.DecodeQuotaRequest(data)
	if err != nil {
		return invalidBody(err)
	}
	if err := e.configures(r); err != nil {
		return err
	}
	entry, err := e.scoped(r)
	if err != nil {
		return err
	}
	built, err := e.accountProvider(r, *entry, request.CredentialID)
	if err != nil {
		return err
	}
	quota := built.Quota()
	if quota == nil {
		writeJSON(w, 200, webapi.QuotaAnswer{})
		return nil
	}
	snapshot, err := quota.Probe(r.Context())
	if err != nil {
		switch {
		case errors.Is(err, provider.ErrQuotaUnsupported):
			snapshot = quota.Latest()
		case errors.Is(err, provider.ErrQuotaRequiresInference):
			return apiFailure(
				409,
				"quota_requires_inference",
				"This provider cannot read its usage without an inference request",
			)
		default:
			return apiFailure(502, "quota_unavailable", err.Error())
		}
	}
	writeJSON(w, 200, webapi.QuotaAnswer{Quota: snapshot})
	return nil
}

func (e *Edge) testProvider(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeTestRequest)
	if err != nil {
		return err
	}
	if err := e.configures(r); err != nil {
		return err
	}
	entry, err := e.scoped(r)
	if err != nil {
		return err
	}
	reservation, err := e.reserve(entry)
	if err != nil {
		return err
	}
	defer reservation.Release()
	built, err := e.accountProvider(r, *entry, request.CredentialID)
	if err != nil {
		return err
	}
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	result, err := shard.TestProvider(r.Context(), *entry, built, request.CredentialID, request.ModelID)
	if err != nil {
		return err
	}
	writeJSON(w, 200, result)
	return nil
}

func patchedAPIKey(entry *providers.ProviderEntry, patch webapi.ProviderPatch) (*providers.APIKeyConfig, error) {
	if patch.APIKey == nil && patch.BaseURL == nil && patch.Models == nil {
		return nil, nil
	}
	old, ok := entry.Credential.(*providers.APIKeyConfig)
	if !ok {
		return nil, apiFailure(400, "subscription_only", "A subscription entry takes only a new label")
	}
	next := *old
	config := &next
	if patch.APIKey != nil {
		secret, err := provider.NewSecret(*patch.APIKey)
		if err != nil {
			return nil, apiFailure(400, "invalid_body", "apiKey: "+err.Error())
		}
		config.APIKey = secret
	}
	if patch.BaseURL != nil {
		config.BaseURL = *patch.BaseURL
	}
	if patch.Models != nil {
		config.Models = *patch.Models
	}
	return config, nil
}

func vendorEndpoint(config *providers.APIKeyConfig, vendor *webapi.Vendor, vendorID string) error {
	if config.BaseURL == nil && vendor.BaseURL != nil {
		endpoint, err := webapi.ParseEndpointURL(*vendor.BaseURL)
		if err != nil {
			return apiFailure(
				502,
				"catalog_unavailable",
				"models.dev names no usable endpoint for "+vendorID,
			)
		}
		config.BaseURL = &endpoint
	}
	return nil
}
