package provider

import (
	"context"
	"errors"

	"github.com/wspl/demi/internal/core"
)

// SubscriptionAccounts is the account operations of one subscription entry.
type SubscriptionAccounts interface {
	Capability() AccountsCapability
	List(context.Context) ([]core.AccountInfo, error)
	Active(context.Context) (*string, error)
	SetActive(context.Context, string) error
	Login(context.Context, func(core.LoginPending)) (core.AccountInfo, error)
	Add(context.Context, AddAccount) (core.AccountInfo, error)
	Remove(context.Context, string) error
}

// AccountsCapability names supported ways of adding an account.
type AccountsCapability struct {
	Login bool
	Add   bool
}

// AddAccount is material supplied by the product: a Claude setup token.
type AddAccount struct{ SetupToken Secret }

// AccountsError reports invalid supplied material or a failed account store.
type AccountsError struct {
	Message string
	Err     error
}

func (e *AccountsError) Error() string {
	if e.Message != "" {
		return e.Message
	}
	return e.Err.Error()
}
func (e *AccountsError) Unwrap() error { return e.Err }

// LoginError reports a failed or unavailable device login.
type LoginError struct {
	Unavailable bool
	Message     string
	Err         error
}

func (e *LoginError) Error() string {
	if e.Message != "" {
		return e.Message
	}
	return e.Err.Error()
}
func (e *LoginError) Unwrap() error { return e.Err }

// AccountLabel names an account and its stable identity.
type AccountLabel struct {
	Label       string
	Detail      *string
	IdentityKey *string
}

// NewAccount holds a family's secret document and public label.
type NewAccount struct {
	Secret string
	Label  AccountLabel
}

// AccountKit supplies the vendor-specific parts of shared account operations.
// Unsupported operations return ErrLoginUnsupported or ErrAccountsUnsupported.
type AccountKit interface {
	Capability() AccountsCapability
	Login(context.Context, func(core.LoginPending)) (NewAccount, error)
	Add(AddAccount) (NewAccount, error)
}

// Accounts implements shared subscription operations over an entry's pool.
type Accounts struct {
	pool  CredentialPool
	kit   AccountKit
	clock core.Clock
}

// NewAccounts connects an entry's pool to its family kit.
func NewAccounts(pool CredentialPool, kit AccountKit, clock core.Clock) *Accounts {
	return &Accounts{pool: pool, kit: kit, clock: clock}
}

// Capability reports the kit's supported account operations.
func (a *Accounts) Capability() AccountsCapability { return a.kit.Capability() }

// List returns accounts in ID order.
func (a *Accounts) List(ctx context.Context) ([]core.AccountInfo, error) {
	accounts, err := a.pool.List(ctx)
	if err != nil {
		return nil, err
	}
	result := make([]core.AccountInfo, len(accounts))
	for i, account := range accounts {
		result[i] = account.Info()
	}
	return result, nil
}

// Active returns the selected account ID.
func (a *Accounts) Active(ctx context.Context) (*string, error) { return a.pool.Active(ctx) }

// SetActive selects an existing account.
func (a *Accounts) SetActive(ctx context.Context, id string) error { return a.pool.SetActive(ctx, id) }

// Login runs the device flow and imports the result only after it completes.
func (a *Accounts) Login(ctx context.Context, pending func(core.LoginPending)) (core.AccountInfo, error) {
	account, err := a.kit.Login(ctx, pending)
	if err != nil {
		return core.AccountInfo{}, err
	}
	if err := ctx.Err(); err != nil {
		return core.AccountInfo{}, err
	}
	info, err := a.importAccount(ctx, account, "login:device")
	if err != nil {
		return core.AccountInfo{}, &LoginError{Err: err}
	}
	return info, nil
}

// Add imports material supplied by the product.
func (a *Accounts) Add(ctx context.Context, input AddAccount) (core.AccountInfo, error) {
	account, err := a.kit.Add(input)
	if err != nil {
		return core.AccountInfo{}, err
	}
	return a.importAccount(ctx, account, "add")
}

// Remove refuses the active account and deletes an existing inactive account.
func (a *Accounts) Remove(ctx context.Context, id string) error {
	active, err := a.pool.Active(ctx)
	if err != nil {
		return err
	}
	if active != nil && *active == id {
		return ErrActiveAccount
	}
	meta, err := a.pool.Meta(ctx, id)
	if err != nil {
		return err
	}
	if meta == nil {
		return &PoolError{ID: id}
	}
	return a.pool.Remove(ctx, id)
}

// importAccount replaces matching identities and selects an entry's first account.
func (a *Accounts) importAccount(ctx context.Context, account NewAccount, source string) (core.AccountInfo, error) {
	var existing *AccountMeta
	if account.Label.IdentityKey != nil {
		var err error
		existing, err = FindByIdentity(ctx, a.pool, *account.Label.IdentityKey)
		if err != nil {
			return core.AccountInfo{}, err
		}
	}
	id := CredentialIDFor(account.Label.IdentityKey, account.Label.Label)
	if existing != nil {
		id = existing.ID
	}
	meta := AccountMeta{ID: id, Label: account.Label.Label, Detail: account.Label.Detail, UpdatedAt: a.clock.Now(), Source: source, IdentityKey: account.Label.IdentityKey}
	if err := a.pool.Write(ctx, meta, account.Secret); err != nil {
		return core.AccountInfo{}, err
	}
	active, err := a.pool.Active(ctx)
	if err != nil {
		return core.AccountInfo{}, err
	}
	if active == nil {
		if err := a.pool.SetActive(ctx, id); err != nil {
			return core.AccountInfo{}, err
		}
	}
	return meta.Info(), nil
}

// AuthReason distinguishes missing, corrupt, inaccessible and refused credentials.
type AuthReason uint8

// Authentication failure reasons shared by provider operations.
const (
	AuthReasonMissing AuthReason = iota
	AuthReasonInvalid
	AuthReasonStore
	AuthReasonRefresh
)

// AuthFailure describes why a family's account could not be used, without tokens.
type AuthFailure struct {
	Family string
	Reason AuthReason
	Detail string
}

// AccountAuthFailure classifies a failed account read or renewal.
func AccountAuthFailure(family string, err error) AuthFailure {
	result := AuthFailure{Family: family, Reason: AuthReasonStore, Detail: err.Error()}
	var account *AccountError
	var renewal *RenewError
	if errors.As(err, &account) {
		switch account.Kind {
		case AccountMissing:
			result.Reason = AuthReasonMissing
		case AccountInvalid:
			result.Reason = AuthReasonInvalid
			result.Detail = "its secret document is " + account.Err.Error()
		case AccountStore:
			result.Detail = account.Err.Error()
		}
	} else if errors.As(err, &renewal) {
		result.Reason = AuthReasonRefresh
	}
	return result
}
func (f AuthFailure) Error() string {
	switch f.Reason {
	case AuthReasonMissing:
		return "No " + f.Family + " account is signed in"
	case AuthReasonInvalid:
		return "The " + f.Family + " account cannot be read: " + f.Detail
	case AuthReasonStore:
		return "The " + f.Family + " account could not be loaded: " + f.Detail
	default:
		return f.Detail
	}
}

// Failure returns the run's authentication failure, with no code for store errors.
func (f AuthFailure) Failure() Failure {
	result := Failure{Message: f.Error()}
	var code ErrorCode
	switch f.Reason {
	case AuthReasonMissing:
		code = AuthMissing
	case AuthReasonInvalid:
		code = AuthInvalid
	case AuthReasonRefresh:
		code = AuthRefreshFailed
	case AuthReasonStore:
		return result
	}
	result.Code = &code
	return result
}

// State returns unauthenticated when missing, or an error for other failures.
func (f AuthFailure) State() core.AuthState {
	message := f.Error()
	if f.Reason == AuthReasonMissing {
		return &core.Unauthenticated{Message: &message}
	}
	return &core.AuthError{Message: message}
}

// QuotaError classifies store failures as unavailable and others as unauthenticated.
func (f AuthFailure) QuotaError() *QuotaError {
	kind := QuotaUnauthenticated
	if f.Reason == AuthReasonStore {
		kind = QuotaUnavailable
	}
	return &QuotaError{Kind: kind, Message: f.Error()}
}

// CatalogError uses the same authentication rule as quota probes.
func (f AuthFailure) CatalogError() *CatalogError {
	kind := CatalogUnauthenticated
	if f.Reason == AuthReasonStore {
		kind = CatalogUnavailable
	}
	return &CatalogError{Kind: kind, Message: f.Error()}
}
