package providerhost

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"reflect"
	"strings"
	"sync"
	"unicode/utf8"

	"github.com/google/uuid"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// Vault is the credential vault. Share its pointer across owners.
type Vault struct {
	control *database.ControlService
	key     *VaultKey
	mode    webapiproto.InstanceMode
	sync    *pagesync.Registry
	gates   provider.RefreshGates
	// mu protects the cached master identity only, never database reads.
	mu     sync.Mutex
	master *webapiproto.UserID
}

// Entry is a provider entry, read and decoded.
type Entry struct {
	// ID identifies the provider entry.
	ID webapiproto.ProviderID
	// Owner identifies the user whose scope owns the entry.
	Owner webapiproto.UserID
	// The entry's family, such as `anthropic` or `codex`.
	Family string
	// Label is the user’s display name for the entry.
	Label string
	// Credential supplies the configured credential kind and values.
	Credential EntryCredential
	// CreatedAt records when the entry was created.
	CreatedAt types.Timestamp
}

// EntryCredential is how an entry authenticates.
type EntryCredential interface{ entryCredential() }

// SubscriptionCredential holds the active account of a subscription entry.
type SubscriptionCredential struct {
	// Active identifies the selected subscription account, or is nil.
	Active *webapiproto.CredentialID
}

func (*SubscriptionCredential) entryCredential() {}
func (*APIKeyConfig) entryCredential()           {}

// An API-key entry's configuration: the document the vault seals. A field
// it does not name is an error, so a misspelled setting is reported rather
// than ignored.
// +demi:root
//
//nolint:revive // The doc comment is the schema's product text, not a Go doc sentence.
type APIKeyConfig struct {
	APIKey  provider.Secret          `json:"apiKey"`
	BaseURL *webapiproto.EndpointURL `json:"baseUrl,omitempty"`
	WireAPI *types.WireAPI           `json:"wireApi,omitempty"`
	// The models.dev vendor the entry was added from.
	// +demi:length chars min=1
	VendorID *string                       `json:"vendorId,omitempty"`
	Models   *webapiproto.ConfiguredModels `json:"models,omitempty"`
}

// Kind returns the entry credential kind.
func (e Entry) Kind() webapiproto.CredentialKind {
	if _, ok := e.Credential.(*APIKeyConfig); ok {
		return webapiproto.CredentialKindAPIKey
	}
	return webapiproto.CredentialKindSubscription
}

// Active returns a subscription entry's active account.
func (e Entry) Active() *webapiproto.CredentialID {
	if c, ok := e.Credential.(*SubscriptionCredential); ok {
		return c.Active
	}
	return nil
}

// DTO returns the entry as the web app sees it: never its key.
func (e Entry) DTO() webapiproto.ProviderDTO {
	dto := webapiproto.ProviderDTO{
		ID:           e.ID,
		Kind:         e.Kind(),
		ProviderType: e.Family,
		Label:        e.Label,
		CreatedAt:    e.CreatedAt,
	}
	if c, ok := e.Credential.(*APIKeyConfig); ok {
		dto.WireAPI = c.WireAPI
		dto.VendorID = c.VendorID
		dto.BaseURL = c.BaseURL
		dto.Models = c.Models
	}
	return dto
}

// NewVault creates a vault with instance scope and page notifications.
func NewVault(
	control *database.ControlService,
	key *VaultKey,
	mode webapiproto.InstanceMode,
	sync *pagesync.Registry,
) *Vault {
	return &Vault{
		control: control,
		key:     key,
		mode:    mode,
		sync:    sync,
	}
}

// Control returns the control store.
func (v *Vault) Control() *database.ControlService {
	return v.control
}

// Key returns the vault encryption key.
func (v *Vault) Key() *VaultKey {
	return v.key
}

// Gates returns the account refresh gates.
func (v *Vault) Gates() *provider.RefreshGates {
	return &v.gates
}

// Configures reports whether user configures entries of their scope.
func (v *Vault) Configures(user webapiproto.UserDTO) bool {
	return v.mode == webapiproto.InstanceModeIsolated || user.Role == webapiproto.RoleMaster
}

// OwnerFor resolves whose entries user infers with.
func (v *Vault) OwnerFor(ctx context.Context, user webapiproto.UserID) (webapiproto.UserID, error) {
	if v.mode == webapiproto.InstanceModeIsolated {
		return user, nil
	}
	v.mu.Lock()
	cached := v.master
	v.mu.Unlock()
	if cached != nil {
		return *cached, nil
	}
	master, found, err := v.control.Master(ctx)
	if err != nil {
		return "", err
	}
	if !found {
		return "", database.CorruptValue("users", "role", errors.New("a shared instance has no master account"))
	}
	v.mu.Lock()
	v.master = &master
	v.mu.Unlock()
	return master, nil
}

// MarkChanged marks providers changed for every user who infers with owner entries.
func (v *Vault) MarkChanged(owner webapiproto.UserID) {
	if v.mode == webapiproto.InstanceModeShared {
		v.sync.MarkEveryone(pagesync.Part{Kind: pagesync.Providers})
	} else {
		v.sync.Mark(owner, pagesync.Part{Kind: pagesync.Providers})
	}
}

// MarkEntryChanged marks the entry changed; lookup failures are logged.
func (v *Vault) MarkEntryChanged(ctx context.Context, id webapiproto.ProviderID) {
	if v.mode == webapiproto.InstanceModeShared {
		v.sync.MarkEveryone(pagesync.Part{Kind: pagesync.Providers})
		return
	}
	row, found, err := v.control.Provider(ctx, id)
	if err != nil {
		slog.Warn("a change of an entry was not marked on its owner's pages", "provider", id, "error", err)
		return
	}
	if found {
		v.sync.Mark(row.Owner, pagesync.Part{Kind: pagesync.Providers})
	}
}

// Visible returns the entry when it belongs to user scope, or nil.
func (v *Vault) Visible(
	ctx context.Context,
	user webapiproto.UserID,
	id webapiproto.ProviderID,
) (*Entry, error) {
	owner, err := v.OwnerFor(ctx, user)
	if err != nil {
		return nil, err
	}
	e, err := v.Entry(ctx, id)
	if err != nil {
		return nil, err
	}
	if e != nil && e.Owner != owner {
		return nil, nil
	}
	return e, nil
}

// Entries returns owner entries, oldest first.
func (v *Vault) Entries(ctx context.Context, owner webapiproto.UserID) ([]Entry, error) {
	rows, err := v.control.Providers(ctx, owner)
	if err != nil {
		return nil, err
	}
	entries := make([]Entry, 0, len(rows))
	for _, row := range rows {
		e, err := v.decode(row)
		if err != nil {
			return nil, err
		}
		entries = append(entries, e)
	}
	return entries, nil
}

// Entry reads and decodes an entry, or returns nil.
func (v *Vault) Entry(ctx context.Context, id webapiproto.ProviderID) (*Entry, error) {
	row, found, err := v.control.Provider(ctx, id)
	if err != nil || !found {
		return nil, err
	}
	e, err := v.decode(row)
	if err != nil {
		return nil, err
	}
	return &e, nil
}

// CreateAPIKey stores a new API-key entry.
func (v *Vault) CreateAPIKey(
	ctx context.Context,
	owner webapiproto.UserID,
	family, label string,
	config APIKeyConfig,
) (Entry, error) {
	randomID, err := uuid.NewRandom()
	if err != nil {
		return Entry{}, fmt.Errorf("create provider identity: %w", err)
	}
	id, err := webapiproto.ParseProviderID(randomID.String())
	if err != nil {
		return Entry{}, err
	}
	sealed, err := v.sealConfig(id, config)
	if err != nil {
		return Entry{}, err
	}
	ctx = context.WithoutCancel(ctx)
	row, err := v.control.InsertProvider(
		ctx,
		database.NewProvider{
			ID:     id,
			Owner:  owner,
			Family: family,
			Kind:   webapiproto.CredentialKindAPIKey,
			Label:  label,
			Config: &sealed,
		},
		nil,
	)
	if err != nil {
		return Entry{}, err
	}
	v.MarkChanged(row.Owner)
	return v.decode(row)
}

// CreateSubscription publishes the entry and staged accounts atomically;
// database.ErrSubscriptionExists means it already exists.
func (v *Vault) CreateSubscription(
	ctx context.Context,
	owner webapiproto.UserID,
	family, label string,
	staged *provider.MemoryCredentialPool,
) (Entry, error) {
	randomID, err := uuid.NewRandom()
	if err != nil {
		return Entry{}, fmt.Errorf("create provider identity: %w", err)
	}
	id, err := webapiproto.ParseProviderID(randomID.String())
	if err != nil {
		return Entry{}, err
	}
	accounts, err := v.sealStagedAccounts(id, staged)
	if err != nil {
		return Entry{}, err
	}
	selected, ok, err := staged.Active(ctx)
	if err != nil {
		return Entry{}, err
	}
	var active *webapiproto.CredentialID
	for i := range accounts {
		if ok && string(accounts[i].ID) == selected {
			active = &accounts[i].ID
			break
		}
	}
	if active == nil && len(accounts) > 0 {
		active = &accounts[0].ID
	}
	ctx = context.WithoutCancel(ctx)
	row, err := v.control.InsertProvider(
		ctx,
		database.NewProvider{
			ID:     id,
			Owner:  owner,
			Family: family,
			Kind:   webapiproto.CredentialKindSubscription,
			Label:  label,
			Active: active,
		},
		accounts,
	)
	if err != nil {
		return Entry{}, err
	}
	v.MarkChanged(row.Owner)
	return v.decode(row)
}

// Update replaces label or configuration; database.ErrProviderNotFound means the entry no longer exists.
func (v *Vault) Update(
	ctx context.Context,
	id webapiproto.ProviderID,
	label *string,
	config *APIKeyConfig,
) (Entry, error) {
	var sealed *[]byte
	if config != nil {
		b, err := v.sealConfig(id, *config)
		if err != nil {
			return Entry{}, err
		}
		sealed = &b
	}
	ctx = context.WithoutCancel(ctx)
	row, err := v.control.UpdateProvider(ctx, id, label, sealed)
	if err != nil {
		return Entry{}, err
	}
	v.MarkChanged(row.Owner)
	return v.decode(row)
}

// Delete deletes the entry and its accounts.
func (v *Vault) Delete(ctx context.Context, entry Entry) error {
	ctx = context.WithoutCancel(ctx)
	if err := v.control.DeleteProvider(ctx, entry.ID); err != nil {
		return err
	}
	v.MarkChanged(entry.Owner)
	return nil
}

// Pool returns the credential pool bound to an entry.
func (v *Vault) Pool(id webapiproto.ProviderID) provider.CredentialPool {
	return NewVaultCredentialPool(v, id)
}

// Accounts reads an entry account records.
func (v *Vault) Accounts(ctx context.Context, id webapiproto.ProviderID) ([]database.CredentialRow, error) {
	return v.control.Credentials(ctx, id)
}

// Account reads an account record, or nil.
func (v *Vault) Account(
	ctx context.Context,
	id webapiproto.ProviderID,
	account webapiproto.CredentialID,
) (database.CredentialRow, bool, error) {
	return v.control.Credential(ctx, id, account)
}

// SealSecret seals a secret document for its account row.
func (v *Vault) SealSecret(id webapiproto.ProviderID, account webapiproto.CredentialID, secret string) ([]byte, error) {
	return v.key.Seal(SecretRow{Provider: id, Account: account}, []byte(secret))
}

// sealConfig serializes the single configuration contract before encrypting it.
func (v *Vault) sealConfig(id webapiproto.ProviderID, config APIKeyConfig) ([]byte, error) {
	document, err := contract.EncodeJSON(config)
	if err != nil {
		return nil, fmt.Errorf("encode provider configuration: %w", err)
	}
	return v.key.Seal(ConfigRow{Provider: id}, document)
}

// decode opens a stored provider configuration without exposing secret values in errors.
func (v *Vault) decode(row database.ProviderRow) (Entry, error) {
	var credential EntryCredential
	switch {
	case row.Kind == webapiproto.CredentialKindAPIKey && row.Config != nil:
		document, err := v.key.Open(ConfigRow{Provider: row.ID}, *row.Config)
		if err != nil {
			return Entry{}, corruptConfig(err.Error())
		}
		if !utf8.Valid(document) {
			return Entry{}, corruptConfig("the configuration is not UTF-8")
		}
		config, err := provider.DecodeSecretDocument(string(document), DecodeAPIKeyConfig)
		if err != nil {
			return Entry{}, corruptConfig(configFault(err))
		}
		credential = &config
	case row.Kind == webapiproto.CredentialKindSubscription && row.Config == nil:
		credential = &SubscriptionCredential{Active: row.Active}
	default:
		return Entry{}, corruptConfig("the configuration does not match the entry's kind")
	}
	return Entry{
		ID:         row.ID,
		Owner:      row.Owner,
		Family:     row.Family,
		Label:      row.Label,
		Credential: credential,
		CreatedAt:  row.CreatedAt,
	}, nil
}

func corruptConfig(reason string) error {
	return database.CorruptValue("providers", "config", errors.New(reason))
}

// configFault only discloses field names declared by the vault contract. Unknown
// keys are untrusted text and can themselves contain a credential.
func configFault(err error) string {
	var fault *provider.SecretDecodeError
	if !errors.As(err, &fault) {
		return "the configuration cannot be read"
	}
	safe := *fault
	safe.Path = "."
	root := fault.Path
	if i := strings.IndexAny(root, ".[ "); i >= 0 {
		root = root[:i]
	}
	shape := reflect.TypeFor[APIKeyConfig]()
	for i := 0; i < shape.NumField(); i++ {
		field := shape.Field(i)
		tag := field.Tag.Get("json")
		name, _, _ := strings.Cut(tag, ",")
		if root == name {
			safe.Path = name
			break
		}
	}
	return safe.Error()
}

// sealStagedAccounts validates and seals the accounts a subscription login staged.
func (v *Vault) sealStagedAccounts(
	id webapiproto.ProviderID,
	staged *provider.MemoryCredentialPool,
) ([]database.CredentialWrite, error) {
	accounts := []database.CredentialWrite{}
	for _, a := range staged.Entries() {
		account, err := webapiproto.ParseCredentialID(a.Meta.ID)
		if err != nil {
			return nil, database.CorruptValue(
				"provider_credentials",
				"id",
				errors.New("a login staged an account whose id is invalid"),
			)
		}
		sealed, err := v.SealSecret(id, account, a.Secret)
		if err != nil {
			return nil, err
		}
		accounts = append(
			accounts,
			database.CredentialWrite{
				ID:          account,
				IdentityKey: a.Meta.IdentityKey,
				Label:       a.Meta.Label,
				Detail:      a.Meta.Detail,
				Source:      a.Meta.Source,
				Secret:      sealed,
			},
		)
	}
	return accounts, nil
}
