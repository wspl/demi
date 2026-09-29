package backend

import (
	"context"
	"log/slog"
	"slices"
	"sync/atomic"
	"unicode/utf8"

	"github.com/google/uuid"

	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/webapi"
)

// Vault is the credential vault (providers.md § Credential vault, § Scope):
// each provider entry's record with its configuration sealed, whose entries
// a user infers with, and the account records of a subscription entry. The
// vault opens a configuration when it reads an entry and validates it
// strictly; a value that does not open or decode is corrupt and is never
// repaired. It is safe for use by any goroutine.
type Vault struct {
	control *storage.Control
	key     VaultKey
	mode    webapi.InstanceMode
	// master is the master's id once read, whose entries a shared
	// instance's users infer with: setup creates it once and it never
	// changes.
	master atomic.Pointer[webapi.UserID]
	// gates let one refresh at a time run for each account of every entry.
	gates *provider.RefreshGates
	// sync is where each change of an entry is marked for the pages.
	sync *SyncRegistry
}

// NewVault makes the vault of the control database control, sealing under
// key, on an instance of mode.
func NewVault(control *storage.Control, key VaultKey, mode webapi.InstanceMode, sync *SyncRegistry) *Vault {
	return &Vault{control: control, key: key, mode: mode, gates: provider.NewRefreshGates(), sync: sync}
}

// ProviderEntry is a provider entry, read and decoded.
type ProviderEntry struct {
	ID    webapi.ProviderID
	Owner webapi.UserID
	// Family is the entry's family, such as anthropic or codex.
	Family     string
	Label      string
	Credential EntryCredential
	CreatedAt  core.Timestamp
}

// EntryCredential is how an entry authenticates: an APIKeyConfig or a
// Subscription.
type EntryCredential interface{ entryCredential() }

// Subscription is a subscription entry's credential: accounts in the
// entry's credential records, of which Active is the one the entry infers
// with, if any.
type Subscription struct{ Active *webapi.CredentialID }

// APIKeyConfig is an API-key entry's configuration: the document the vault
// seals. A member it does not name is an error, so a misspelled setting is
// reported rather than ignored.
//
//demi:wire
type APIKeyConfig struct {
	APIKey  provider.Secret     `json:"apiKey" check:"func=provider.Validate"`
	BaseURL *webapi.EndpointURL `json:"baseUrl,omitzero" check:"func=webapi.Validate"`
	WireAPI *core.WireAPI       `json:"wireApi,omitzero" check:"func=core.Validate"`
	// VendorID is the models.dev vendor the entry was added from.
	VendorID *string                  `json:"vendorId,omitzero" check:"bytes=1.."`
	Models   *webapi.ConfiguredModels `json:"models,omitzero" check:"func=webapi.Validate"`
}

func (APIKeyConfig) entryCredential() {}
func (Subscription) entryCredential() {}

// Kind is how the entry authenticates.
func (e ProviderEntry) Kind() webapi.CredentialKind {
	if _, ok := e.Credential.(APIKeyConfig); ok {
		return webapi.CredentialKindAPIKey
	}
	return webapi.CredentialKindSubscription
}

// Active is a subscription entry's active account; nil for none, and for
// an API-key entry.
func (e ProviderEntry) Active() *webapi.CredentialID {
	if subscription, ok := e.Credential.(Subscription); ok {
		return subscription.Active
	}
	return nil
}

// DTO is the entry as the browser sees it: never its key.
func (e ProviderEntry) DTO() webapi.ProviderDTO {
	dto := webapi.ProviderDTO{ID: e.ID, Kind: e.Kind(), ProviderType: e.Family, Label: e.Label, CreatedAt: e.CreatedAt}
	if config, ok := e.Credential.(APIKeyConfig); ok {
		dto.WireAPI = config.WireAPI
		dto.VendorID = config.VendorID
		dto.BaseURL = config.BaseURL
		dto.Models = config.Models
	}
	return dto
}

// Configures says whether user configures the entries of their scope:
// everyone on an isolated instance, only the master on a shared one.
func (v *Vault) Configures(user webapi.UserDTO) bool {
	return v.mode == webapi.InstanceModeIsolated || user.Role == webapi.RoleMaster
}

// OwnerFor is whose entries user infers with (product.md § Instance mode):
// their own on an isolated instance, the master's on a shared one.
func (v *Vault) OwnerFor(ctx context.Context, user webapi.UserID) (webapi.UserID, error) {
	if v.mode == webapi.InstanceModeIsolated {
		return user, nil
	}
	if master := v.master.Load(); master != nil {
		return *master, nil
	}
	master, err := v.control.Master(ctx)
	if err != nil {
		return webapi.UserID{}, err
	}
	if master == nil {
		// A signed-in user means setup ran, and setup creates the master.
		return webapi.UserID{}, &storage.CorruptError{Table: "users", Column: "role", Reason: "a shared instance has no master account"}
	}
	v.master.Store(master)
	return *master, nil
}

// markChanged marks the providers changed on the channels of every user who
// infers with the entries of owner: every user on a shared instance, the
// owner alone on an isolated one.
func (v *Vault) markChanged(owner webapi.UserID) {
	if v.mode == webapi.InstanceModeShared {
		v.sync.MarkEveryone(SyncProviders)
		return
	}
	v.sync.Mark(owner, SyncProviders)
}

// markEntryChanged marks the providers changed for every user who infers
// with the entry id. It runs after the change committed, so ctx's
// cancellation does not stop it. A lookup that fails is logged: the pages
// then show the change with the entry's next one.
func (v *Vault) markEntryChanged(ctx context.Context, id webapi.ProviderID) {
	if v.mode == webapi.InstanceModeShared {
		v.sync.MarkEveryone(SyncProviders)
		return
	}
	row, err := v.control.Provider(context.WithoutCancel(ctx), id)
	if err != nil {
		slog.Warn("a change of an entry was not marked on its owner's pages", "provider", id.String(), "error", err)
		return
	}
	// An entry deleted meanwhile marked its owner as it went.
	if row != nil {
		v.sync.Mark(row.Owner, SyncProviders)
	}
}

// Visible is the entry id when it is one of user's scope, and nil
// otherwise.
func (v *Vault) Visible(ctx context.Context, user webapi.UserID, id webapi.ProviderID) (*ProviderEntry, error) {
	owner, err := v.OwnerFor(ctx, user)
	if err != nil {
		return nil, err
	}
	entry, err := v.Entry(ctx, id)
	if err != nil || entry == nil || entry.Owner != owner {
		return nil, err
	}
	return entry, nil
}

// Entries are owner's entries, oldest first.
func (v *Vault) Entries(ctx context.Context, owner webapi.UserID) ([]ProviderEntry, error) {
	rows, err := v.control.Providers(ctx, owner)
	if err != nil {
		return nil, err
	}
	entries := make([]ProviderEntry, 0, len(rows))
	for _, row := range rows {
		entry, err := v.decode(row)
		if err != nil {
			return nil, err
		}
		entries = append(entries, entry)
	}
	return entries, nil
}

// Entry is the entry id, or nil when there is none.
func (v *Vault) Entry(ctx context.Context, id webapi.ProviderID) (*ProviderEntry, error) {
	row, err := v.control.Provider(ctx, id)
	if err != nil || row == nil {
		return nil, err
	}
	return v.decoded(*row)
}

// CreateAPIKey stores a new API-key entry of owner's.
func (v *Vault) CreateAPIKey(ctx context.Context, owner webapi.UserID, family, label string, config APIKeyConfig) (ProviderEntry, error) {
	id := newProviderID()
	sealed, err := v.sealConfig(id, config)
	if err != nil {
		return ProviderEntry{}, err
	}
	row, err := v.control.InsertProvider(ctx, storage.NewProvider{ID: id, Owner: owner, Family: family, Kind: webapi.CredentialKindAPIKey, Label: label, Config: sealed}, nil)
	if err != nil {
		return ProviderEntry{}, err
	}
	// An API-key entry is under no uniqueness rule, so the insert stored it.
	v.markChanged(row.Owner)
	return v.decode(*row)
}

// CreateSubscription publishes a completed login's entry with the accounts
// of its staged pool, in one transaction (providers.md § Login and
// publication): the entry, its accounts, and the staged active account, or
// its first. It answers nil when owner already holds the family's
// subscription entry, and then stores nothing.
func (v *Vault) CreateSubscription(ctx context.Context, owner webapi.UserID, family, label string, staged *provider.MemoryCredentialPool) (*ProviderEntry, error) {
	id := newProviderID()
	var accounts []storage.CredentialWrite
	for _, drafted := range staged.Entries() {
		account, err := webapi.ParseCredentialID(drafted.Meta.ID)
		if err != nil {
			return nil, &storage.CorruptError{Table: "provider_credentials", Column: "id", Reason: "a login staged an account whose id is invalid: " + err.Error()}
		}
		accounts = append(accounts, storage.CredentialWrite{
			ID:          account,
			IdentityKey: drafted.Meta.IdentityKey,
			Label:       drafted.Meta.Label,
			Detail:      drafted.Meta.Detail,
			Source:      drafted.Meta.Source,
			Secret:      v.sealSecret(id, account, drafted.Secret),
		})
	}
	// A pool held in memory fails only once ctx has ended, and the insert
	// then fails too.
	stagedActive, _ := staged.Active(ctx)
	chosen := slices.IndexFunc(accounts, func(account storage.CredentialWrite) bool {
		return stagedActive != nil && account.ID.String() == *stagedActive
	})
	if chosen < 0 && len(accounts) > 0 {
		chosen = 0
	}
	var active *webapi.CredentialID
	if chosen >= 0 {
		active = &accounts[chosen].ID
	}
	row, err := v.control.InsertProvider(ctx, storage.NewProvider{ID: id, Owner: owner, Family: family, Kind: webapi.CredentialKindSubscription, Label: label, Active: active}, accounts)
	if err != nil || row == nil {
		return nil, err
	}
	v.markChanged(row.Owner)
	return v.decoded(*row)
}

// Update replaces the entry's label or configuration, where not nil; it
// answers nil for an entry that no longer exists.
func (v *Vault) Update(ctx context.Context, id webapi.ProviderID, label *string, config *APIKeyConfig) (*ProviderEntry, error) {
	var sealed []byte
	if config != nil {
		var err error
		sealed, err = v.sealConfig(id, *config)
		if err != nil {
			return nil, err
		}
	}
	row, err := v.control.UpdateProvider(ctx, id, label, sealed)
	if err != nil || row == nil {
		return nil, err
	}
	v.markChanged(row.Owner)
	return v.decoded(*row)
}

// Delete deletes the entry with its accounts and catalog record.
func (v *Vault) Delete(ctx context.Context, entry ProviderEntry) error {
	if err := v.control.DeleteProvider(ctx, entry.ID); err != nil {
		return err
	}
	v.markChanged(entry.Owner)
	return nil
}

// Pool is the pool of entry id's accounts, which can reach no other
// entry's.
func (v *Vault) Pool(id webapi.ProviderID) provider.CredentialPool {
	return &vaultPool{vault: v, provider: id}
}

// Accounts are the entry's account records, ordered by id, their secrets
// unopened.
func (v *Vault) Accounts(ctx context.Context, id webapi.ProviderID) ([]storage.CredentialRow, error) {
	return v.control.Credentials(ctx, id)
}

// Account is the entry's account record account, or nil when there is
// none.
func (v *Vault) Account(ctx context.Context, id webapi.ProviderID, account webapi.CredentialID) (*storage.CredentialRow, error) {
	return v.control.Credential(ctx, id, account)
}

// sealSecret seals the account's secret document for its row.
func (v *Vault) sealSecret(id webapi.ProviderID, account webapi.CredentialID, secret string) []byte {
	return v.key.Seal(SecretRow(id, account), []byte(secret))
}

// sealConfig seals the entry's configuration for its row.
func (v *Vault) sealConfig(id webapi.ProviderID, config APIKeyConfig) ([]byte, error) {
	document, err := encode(config)
	if err != nil {
		return nil, err
	}
	return v.key.Seal(ConfigRow(id), document), nil
}

// decoded is decode's entry as a pointer.
func (v *Vault) decoded(row storage.ProviderRow) (*ProviderEntry, error) {
	entry, err := v.decode(row)
	if err != nil {
		return nil, err
	}
	return &entry, nil
}

// decode is the entry of row, its configuration opened.
func (v *Vault) decode(row storage.ProviderRow) (ProviderEntry, error) {
	entry := ProviderEntry{ID: row.ID, Owner: row.Owner, Family: row.Family, Label: row.Label, CreatedAt: row.CreatedAt}
	switch {
	case row.Kind == webapi.CredentialKindAPIKey && row.Config != nil:
		config, err := v.openConfig(row.ID, row.Config)
		if err != nil {
			return ProviderEntry{}, err
		}
		entry.Credential = config
	case row.Kind == webapi.CredentialKindSubscription && row.Config == nil:
		entry.Credential = Subscription{Active: row.Active}
	default:
		// The schema ties the configuration to the kind.
		return ProviderEntry{}, corruptConfig("the configuration does not match the entry's kind")
	}
	return entry, nil
}

// openConfig is the configuration sealed for entry id: it must open, be
// UTF-8 JSON of the configuration's shape, and keep its rules. A failure
// names the member and the kind of fault, never a value, which may be the
// key.
func (v *Vault) openConfig(id webapi.ProviderID, sealed []byte) (APIKeyConfig, error) {
	document, err := v.key.Open(ConfigRow(id), sealed)
	if err != nil {
		return APIKeyConfig{}, corruptConfig(err.Error())
	}
	if !utf8.Valid(document) {
		return APIKeyConfig{}, corruptConfig("the configuration is not UTF-8")
	}
	config, err := provider.DecodeSecret(document, decode[APIKeyConfig])
	if err != nil {
		return APIKeyConfig{}, corruptConfig(err.Error())
	}
	return config, nil
}

// corruptConfig is a configuration that cannot be used, for reason.
func corruptConfig(reason string) error {
	return &storage.CorruptError{Table: "providers", Column: "config", Reason: reason}
}

// newProviderID is a new entry's id.
func newProviderID() webapi.ProviderID {
	id, err := webapi.ParseProviderID(uuid.NewString())
	if err != nil {
		// A UUID is never empty.
		panic(err)
	}
	return id
}
