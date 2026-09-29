package edge

import (
	"errors"
	"net/http"
	"slices"

	"github.com/wspl/demi/go/backend"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/webapi"
)

// /api/providers (web-api.md § Model configuration and provider
// inspection): the entries of the caller's scope, the vendors an entry can
// be added from, and each entry's status and quota. Every user of a scope
// reads it; configuring and refreshing usage are the master's on a shared
// instance and each owner's on an isolated one. No answer carries key
// material.

// configures refuses a user who does not configure the scope's entries.
func (s *Server) configures(user webapi.UserDTO) error {
	if !s.services.Vault.Configures(user) {
		return forbidden("Providers are configured by the instance owner")
	}
	return nil
}

// scoped is the entry the path names, when it is one of the caller's
// scope; any other answers like a missing one.
func (s *Server) scoped(r *http.Request, user webapi.UserDTO) (backend.ProviderEntry, error) {
	id, err := webapi.ParseProviderID(r.PathValue("id"))
	if err != nil {
		return backend.ProviderEntry{}, providerNotFound()
	}
	entry, err := s.services.Vault.Visible(r.Context(), user.ID, id)
	if err != nil {
		return backend.ProviderEntry{}, err
	}
	if entry == nil {
		return backend.ProviderEntry{}, providerNotFound()
	}
	return *entry, nil
}

// configuredEntry is the entry the path names, for a user who configures
// the scope's entries.
func (s *Server) configuredEntry(r *http.Request, user webapi.UserDTO) (backend.ProviderEntry, error) {
	if err := s.configures(user); err != nil {
		return backend.ProviderEntry{}, err
	}
	return s.scoped(r, user)
}

// reserve holds the entry for one change; the caller releases it.
func (s *Server) reserve(entry backend.ProviderEntry) (func(), error) {
	release, ok := s.services.Operations.Reserve(entry.ID)
	if !ok {
		return nil, providerBusy()
	}
	return release, nil
}

// scopeEntries are the entries of the caller's scope.
func (s *Server) scopeEntries(r *http.Request) ([]backend.ProviderEntry, error) {
	user, err := caller(r)
	if err != nil {
		return nil, err
	}
	owner, err := s.services.Vault.OwnerFor(r.Context(), user.ID)
	if err != nil {
		return nil, err
	}
	return s.services.Vault.Entries(r.Context(), owner)
}

func (s *Server) listProviders(w http.ResponseWriter, r *http.Request) error {
	entries, err := s.scopeEntries(r)
	if err != nil {
		return err
	}
	providers := make([]webapi.ProviderDTO, 0, len(entries))
	for _, entry := range entries {
		providers = append(providers, entry.DTO())
	}
	writeJSON(w, http.StatusOK, webapi.Providers{Providers: providers})
	return nil
}

// providerCatalog is what the page can add: each subscription family with
// whether the scope holds its entry, and the models.dev vendors a family
// speaks to.
func (s *Server) providerCatalog(w http.ResponseWriter, r *http.Request) error {
	entries, err := s.scopeEntries(r)
	if err != nil {
		return err
	}
	families := s.services.Assembly.Families().Subscriptions()
	subscriptions := make([]webapi.SubscriptionFamily, 0, len(families))
	for _, family := range families {
		configured := slices.ContainsFunc(entries, func(entry backend.ProviderEntry) bool { return entry.Family == family })
		subscriptions = append(subscriptions, webapi.SubscriptionFamily{ProviderType: family, Configured: configured})
	}
	vendors, err := s.services.Assembly.Vendors().Vendors(r.Context())
	if err != nil {
		return catalogUnavailable(err.Error())
	}
	writeJSON(w, http.StatusOK, webapi.VendorCatalog{Subscriptions: subscriptions, Vendors: vendors})
	return nil
}

func catalogUnavailable(message string) *apiError {
	return newError(http.StatusBadGateway, webapi.ErrorCodeCatalogUnavailable, message)
}

func unknownFamily(family string) *apiError {
	return newError(http.StatusBadRequest, webapi.ErrorCodeUnknownProviderType, `Unknown provider type "`+family+`"`)
}

// apiKey is the key a body carries, as a credential: one line of text.
func apiKey(text string) (provider.Secret, error) {
	secret, err := provider.NewSecret(text)
	if err != nil {
		return provider.Secret{}, invalidBody("apiKey: " + err.Error())
	}
	return secret, nil
}

// apiKeyFamily refuses a family that is unknown, takes no API key, or does
// not speak wire when the entry names one.
func (s *Server) apiKeyFamily(name string, wire *core.WireAPI) error {
	family, ok := s.services.Assembly.Families().Get(name)
	if !ok {
		return unknownFamily(name)
	}
	if family.Credential() == webapi.CredentialKindSubscription {
		return newError(http.StatusBadRequest, webapi.ErrorCodeSubscriptionOnly, `Provider type "`+name+`" is configured by its login`)
	}
	if wire != nil && !slices.Contains(family.Wires(), *wire) {
		return invalidBody("wireApi: the " + name + " family does not speak " + string(*wire))
	}
	return nil
}

// createProvider creates an API-key entry, from the vendor list or for a
// custom endpoint.
func (s *Server) createProvider(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	body, err := jsonBody[webapi.CreateProvider](w, r)
	if err != nil {
		return err
	}
	if err := s.configures(user); err != nil {
		return err
	}
	var family, label string
	var config backend.APIKeyConfig
	switch body := body.(type) {
	case webapi.CreateProviderVendor:
		family, config, err = s.vendorConfig(r, body)
		label = string(body.Label)
	case webapi.CreateProviderCustom:
		family, config, err = s.customConfig(body)
		label = string(body.Label)
	}
	if err != nil {
		return err
	}
	owner, err := s.services.Vault.OwnerFor(r.Context(), user.ID)
	if err != nil {
		return err
	}
	entry, err := s.services.Vault.CreateAPIKey(r.Context(), owner, family, label, config)
	if err != nil {
		return err
	}
	writeJSON(w, http.StatusCreated, webapi.ProviderAnswer{Provider: entry.DTO()})
	return nil
}

// vendorConfig is the family and configuration of an entry added from the
// vendor list: the vendor's family, wire and endpoint, unless the body
// names its own endpoint.
func (s *Server) vendorConfig(r *http.Request, body webapi.CreateProviderVendor) (string, backend.APIKeyConfig, error) {
	vendor, err := s.services.Assembly.Vendors().Vendor(r.Context(), body.VendorID)
	if err != nil {
		return "", backend.APIKeyConfig{}, catalogUnavailable(err.Error())
	}
	if vendor == nil {
		return "", backend.APIKeyConfig{}, newError(http.StatusBadRequest, webapi.ErrorCodeUnknownVendor, `Unknown vendor "`+body.VendorID+`"`)
	}
	if err := s.apiKeyFamily(vendor.ProviderType, vendor.WireAPI); err != nil {
		return "", backend.APIKeyConfig{}, err
	}
	baseURL := body.BaseURL
	if baseURL == nil && vendor.BaseURL != nil {
		endpoint, err := webapi.ParseEndpointURL(*vendor.BaseURL)
		if err != nil {
			return "", backend.APIKeyConfig{}, catalogUnavailable("models.dev names no usable endpoint for " + body.VendorID)
		}
		baseURL = &endpoint
	}
	key, err := apiKey(body.APIKey)
	if err != nil {
		return "", backend.APIKeyConfig{}, err
	}
	config := backend.APIKeyConfig{APIKey: key, BaseURL: baseURL, WireAPI: vendor.WireAPI, VendorID: &vendor.ID, Models: body.Models}
	return vendor.ProviderType, config, nil
}

// customConfig is the family and configuration of an entry for a custom
// endpoint.
func (s *Server) customConfig(body webapi.CreateProviderCustom) (string, backend.APIKeyConfig, error) {
	if err := s.apiKeyFamily(body.ProviderType, body.WireAPI); err != nil {
		return "", backend.APIKeyConfig{}, err
	}
	key, err := apiKey(body.APIKey)
	if err != nil {
		return "", backend.APIKeyConfig{}, err
	}
	config := backend.APIKeyConfig{APIKey: key, BaseURL: body.BaseURL, WireAPI: body.WireAPI, Models: body.Models}
	return body.ProviderType, config, nil
}

// updateProvider edits an entry: the label of any, and the key, endpoint
// and model list of an API-key entry. A configuration edit starts the
// entry's provider and catalog afresh.
func (s *Server) updateProvider(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	patch, err := jsonBody[webapi.ProviderPatch](w, r)
	if err != nil {
		return err
	}
	entry, err := s.configuredEntry(r, user)
	if err != nil {
		return err
	}
	release, err := s.reserve(entry)
	if err != nil {
		return err
	}
	defer release()
	var config *backend.APIKeyConfig
	reconfigures := patch.APIKey != nil || patch.BaseURL != nil || patch.Models != nil
	if reconfigures {
		current, ok := entry.Credential.(backend.APIKeyConfig)
		if !ok {
			return newError(http.StatusBadRequest, webapi.ErrorCodeSubscriptionOnly, "A subscription entry takes only a new label")
		}
		if patch.APIKey != nil {
			if current.APIKey, err = apiKey(*patch.APIKey); err != nil {
				return err
			}
		}
		if patch.BaseURL != nil {
			current.BaseURL = *patch.BaseURL
		}
		if patch.Models != nil {
			current.Models = *patch.Models
		}
		config = &current
	}
	var label *string
	if patch.Label != nil {
		label = new(string(*patch.Label))
	}
	updated, err := s.services.Vault.Update(r.Context(), entry.ID, label, config)
	if err != nil {
		return err
	}
	if updated == nil {
		return providerNotFound()
	}
	if reconfigures {
		if err := s.services.Assembly.Invalidate(r.Context(), entry.ID); err != nil {
			return err
		}
	}
	writeJSON(w, http.StatusOK, webapi.ProviderAnswer{Provider: updated.DTO()})
	return nil
}

// deleteProvider deletes an entry with its accounts, catalog and quota
// snapshots.
func (s *Server) deleteProvider(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	entry, err := s.configuredEntry(r, user)
	if err != nil {
		return err
	}
	release, err := s.reserve(entry)
	if err != nil {
		return err
	}
	defer release()
	if err := s.services.Vault.Delete(r.Context(), entry); err != nil {
		return err
	}
	if err := s.services.Assembly.Forget(r.Context(), entry.ID); err != nil {
		return err
	}
	w.WriteHeader(http.StatusNoContent)
	return nil
}

// providerStatus is the entry's health, accounts and quota; reading it
// never probes and never runs inference.
func (s *Server) providerStatus(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	entry, err := s.scoped(r, user)
	if err != nil {
		return err
	}
	details, err := s.services.Assembly.Details(r.Context(), entry, s.services.Vault.Configures(user))
	if err != nil {
		return err
	}
	writeJSON(w, http.StatusOK, details)
	return nil
}

// accountProvider is the entry's provider for account, or for its active
// account when account is nil.
func (s *Server) accountProvider(r *http.Request, entry backend.ProviderEntry, account *webapi.CredentialID) (provider.Provider, error) {
	if account == nil {
		return s.services.Assembly.ProviderFor(r.Context(), entry)
	}
	made, err := s.services.Assembly.ForAccount(r.Context(), entry, *account)
	if err != nil {
		return nil, err
	}
	if made == nil {
		return nil, accountNotFound()
	}
	return made, nil
}

// providerQuota probes the quota of the account the body names, or of the
// active one: only a free probe runs, and a family that cannot probe
// answers its kept snapshot. Cancelling the request stops the probe.
func (s *Server) providerQuota(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	request, err := optionalJSONBody[webapi.QuotaRequest](w, r)
	if err != nil {
		return err
	}
	entry, err := s.configuredEntry(r, user)
	if err != nil {
		return err
	}
	made, err := s.accountProvider(r, entry, request.CredentialID)
	if err != nil {
		return err
	}
	owned, ok := made.(provider.QuotaProvider)
	if !ok || owned.Quota() == nil {
		writeJSON(w, http.StatusOK, webapi.QuotaAnswer{})
		return nil
	}
	quota := owned.Quota()
	snapshot, err := quota.Probe(r.Context())
	switch {
	case errors.Is(err, provider.ErrQuotaUnsupported):
		snapshot = quota.Latest()
	case errors.Is(err, provider.ErrQuotaRequiresInference):
		return newError(http.StatusConflict, webapi.ErrorCodeQuotaRequiresInference, "This provider cannot read its usage without an inference request")
	case err != nil:
		return newError(http.StatusBadGateway, webapi.ErrorCodeQuotaUnavailable, err.Error())
	}
	writeJSON(w, http.StatusOK, webapi.QuotaAnswer{Quota: snapshot})
	return nil
}
