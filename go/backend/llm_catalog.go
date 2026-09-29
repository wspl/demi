package backend

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"slices"
	"sync"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/webapi"
)

// NotConfiguredError is a model the entry's configured list does not name;
// its request fails.
type NotConfiguredError struct{ Model string }

func (e *NotConfiguredError) Error() string {
	return "the model " + e.Model + " is not in the provider's configured list"
}

// configuredModel is a configured model as a catalog model (models.md §
// Catalog sources): it states its facts directly, its first thinking effort
// is its default, its Fast tier is its only service tier, and it is taken
// to call tools.
func configuredModel(model webapi.ConfiguredModel) core.ProviderModel {
	efforts := slices.Clone(model.ThinkingEfforts)
	catalog := core.ProviderModel{
		ID:                       model.ID.String(),
		DisplayName:              model.DisplayName.String(),
		ContextWindow:            new(model.ContextWindow),
		OutputLimit:              model.OutputLimit,
		SupportsTools:            new(true),
		AcceptedExtensions:       model.AcceptedExtensions,
		SupportsReasoning:        new(len(efforts) > 0),
		SupportedThinkingEfforts: &efforts,
		ServiceTiers:             []core.ServiceTier{},
	}
	if model.AcceptedExtensions != nil {
		catalog.SupportsAttachments = new(len(*model.AcceptedExtensions) > 0)
	}
	if len(efforts) > 0 {
		catalog.DefaultThinkingEffort = new(efforts[0])
	}
	if model.FastTier != nil {
		catalog.ServiceTiers = append(catalog.ServiceTiers, core.ServiceTier{ID: *model.FastTier, Label: "Fast", Fast: true})
	}
	return catalog
}

// ConfiguredSelection is selection with the facts of the entry's configured
// model applied again, as every inference boundary applies them (models.md
// § Request parameters): an edit of a model's limits reaches the next
// request, and the thinking and tier stay as the user chose them. An entry
// without a configured list keeps the selection as it is; a model its list
// does not name is a *NotConfiguredError.
func ConfiguredSelection(entry ProviderEntry, selection core.ModelSelection) (core.ModelSelection, error) {
	config, ok := entry.Credential.(APIKeyConfig)
	if !ok || config.Models == nil {
		return selection, nil
	}
	index := slices.IndexFunc(config.Models.Models, func(model webapi.ConfiguredModel) bool {
		return model.ID.String() == selection.Model.ID
	})
	if index < 0 {
		return core.ModelSelection{}, &NotConfiguredError{selection.Model.ID}
	}
	return configuredModel(config.Models.Models[index]).Selection(entry.ID.String(), selection.Thinking, selection.ServiceTierID), nil
}

// unfetched is a list never fetched: a configured list, or an empty
// catalog with its warnings.
func unfetched(models []core.ProviderModel, warnings []string) core.ProviderModelList {
	return core.ProviderModelList{Models: models, Warnings: warnings, SourceFetchedAt: core.UnixEpoch}
}

// EntryCatalog is the entry's catalog, from the first source it has: its
// configured list, its vendor's list, or its provider's directory. A failed
// read of the source is the catalog's warning: the last good copy stays,
// and without one the catalog is empty. made is the entry's provider, or
// buildErr why it could not be built.
func (a *ProviderAssembly) EntryCatalog(ctx context.Context, entry ProviderEntry, made provider.Provider, buildErr error, refresh bool) core.ProviderModelList {
	var vendor *string
	if config, ok := entry.Credential.(APIKeyConfig); ok {
		if config.Models != nil {
			models := make([]core.ProviderModel, 0, len(config.Models.Models))
			for _, model := range config.Models.Models {
				models = append(models, configuredModel(model))
			}
			return unfetched(models, []string{})
		}
		vendor = config.VendorID
	}
	var fetch CatalogFetch
	switch {
	case vendor != nil:
		fetch = func(ctx context.Context) (core.ProviderModelList, error) { return a.vendors.Models(ctx, *vendor) }
	case buildErr != nil:
		return unfetched([]core.ProviderModel{}, []string{buildErr.Error()})
	default:
		fetch = made.ListModels
	}
	catalog, err := a.catalogs.Read(ctx, entry.ID, catalogKey(entry), fetch, refresh)
	if err != nil {
		return unfetched([]core.ProviderModel{}, []string{err.Error()})
	}
	return catalog
}

// ModelCatalog is the catalog of every entry in entries, each with its
// provider's health: one entry's failure leaves the others intact.
func (a *ProviderAssembly) ModelCatalog(ctx context.Context, entries []ProviderEntry, refresh bool) []webapi.CatalogProvider {
	catalogs := make([]webapi.CatalogProvider, len(entries))
	var read sync.WaitGroup
	for i, entry := range entries {
		read.Go(func() { catalogs[i] = a.catalogProvider(ctx, entry, refresh) })
	}
	read.Wait()
	return catalogs
}

func (a *ProviderAssembly) catalogProvider(ctx context.Context, entry ProviderEntry, refresh bool) webapi.CatalogProvider {
	made, buildErr := a.ProviderFor(ctx, entry)
	catalog := a.EntryCatalog(ctx, entry, made, buildErr, refresh)
	auth, runtime := a.Health(ctx, entry, made, buildErr)
	models := make([]webapi.CatalogModel, 0, len(catalog.Models))
	for _, model := range catalog.Models {
		models = append(models, webapi.CatalogModel{ProviderModel: model, Selection: model.Selection(entry.ID.String(), nil, nil), UnnamedEffort: model.UnnamedEffort()})
	}
	return webapi.CatalogProvider{
		ProviderID:                 entry.ID,
		DisplayName:                entry.Label,
		RequiresProcessCapableHost: buildErr == nil && made.Capabilities().ProcessHost,
		Models:                     models,
		SourceFetchedAt:            catalog.SourceFetchedAt,
		Stale:                      catalog.Stale,
		Warnings:                   catalog.Warnings,
		Auth:                       auth,
		Runtime:                    runtime,
		Availability:               availability(auth, runtime),
	}
}

// catalogIdentity is what a catalog record's key digests: the entry's
// family, configuration and active account.
type catalogIdentity struct {
	Family  string         `json:"family"`
	Config  jsontext.Value `json:"config"`
	Account *string        `json:"account"`
}

// catalogKey is the digest a catalog record is kept under: the SHA-256 of
// the RFC 8785 form of the entry's family, configuration and active
// account, so a changed entry starts from an empty cache (storage.md §
// Encodings and digests).
func catalogKey(entry ProviderEntry) string {
	identity := catalogIdentity{Family: entry.Family, Config: jsontext.Value("null")}
	if config, ok := entry.Credential.(APIKeyConfig); ok {
		document, err := encode(config)
		if err != nil {
			// A configuration the vault decoded encodes.
			panic(err)
		}
		identity.Config = document
	}
	if active := entry.Active(); active != nil {
		identity.Account = new(active.String())
	}
	document, err := json.Marshal(identity)
	if err != nil {
		// Strings and a JSON document always encode.
		panic(err)
	}
	canonical := jsontext.Value(document)
	if err := canonical.Canonicalize(); err != nil {
		panic(err)
	}
	digest := sha256.Sum256(canonical)
	return hex.EncodeToString(digest[:])
}
