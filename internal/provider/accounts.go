package provider

import (
	"context"
	"errors"
	"fmt"

	"github.com/wspl/demi/internal/types"
)

// SubscriptionAccounts is the account operations of one subscription entry.
type SubscriptionAccounts interface {
	Capability() AccountsCapability
	List(context.Context) ([]types.AccountInfo, error)
	Active(context.Context) (string, bool, error)
	SetActive(context.Context, string) error
	Login(context.Context, func(types.LoginPending)) (types.AccountInfo, error)
	Add(context.Context, AddAccount) (types.AccountInfo, error)
	Remove(context.Context, string) error
}

// AccountsCapability names supported ways of adding an account.
type AccountsCapability struct {
	Login bool
	Add   bool
}

// AddAccount is material supplied by the product: a Claude setup token.
type AddAccount struct{ SetupToken Secret }

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
	Login(context.Context, func(types.LoginPending)) (NewAccount, error)
	Add(AddAccount) (NewAccount, error)
}

// Accounts implements shared subscription operations over an entry's pool.
type Accounts struct {
	pool  CredentialPool
	kit   AccountKit
	clock types.Clock
}

// NewAccounts connects an entry's pool to its family kit.
func NewAccounts(pool CredentialPool, kit AccountKit, clock types.Clock) *Accounts {
	return &Accounts{pool: pool, kit: kit, clock: clock}
}

// Capability reports the kit's supported account operations.
func (a *Accounts) Capability() AccountsCapability {
	return a.kit.Capability()
}

// List returns accounts in ID order.
func (a *Accounts) List(ctx context.Context) ([]types.AccountInfo, error) {
	accounts, err := a.pool.List(ctx)
	if err != nil {
		return nil, err
	}
	result := make([]types.AccountInfo, len(accounts))
	for i, account := range accounts {
		result[i] = account.Info()
	}
	return result, nil
}

// Active returns the selected account ID; ok is false when there is none.
func (a *Accounts) Active(ctx context.Context) (string, bool, error) {
	return a.pool.Active(ctx)
}

// SetActive selects an existing account.
func (a *Accounts) SetActive(ctx context.Context, id string) error {
	return a.pool.SetActive(ctx, id)
}

// Login runs the device flow and imports the result only after it completes.
func (a *Accounts) Login(ctx context.Context, pending func(types.LoginPending)) (types.AccountInfo, error) {
	account, err := a.kit.Login(ctx, pending)
	if err != nil {
		return types.AccountInfo{}, err
	}
	if err := ctx.Err(); err != nil {
		return types.AccountInfo{}, err
	}
	info, err := a.importAccount(ctx, account, "login:device")
	if err != nil {
		return types.AccountInfo{}, err
	}
	return info, nil
}

// Add imports material supplied by the product.
func (a *Accounts) Add(ctx context.Context, input AddAccount) (types.AccountInfo, error) {
	account, err := a.kit.Add(input)
	if err != nil {
		return types.AccountInfo{}, err
	}
	return a.importAccount(ctx, account, "add")
}

// Remove refuses the active account and deletes an existing inactive account.
func (a *Accounts) Remove(ctx context.Context, id string) error {
	active, ok, err := a.pool.Active(ctx)
	if err != nil {
		return err
	}
	if ok && active == id {
		return ErrActiveAccount
	}
	_, ok, err = a.pool.Meta(ctx, id)
	if err != nil {
		return err
	}
	if !ok {
		return fmt.Errorf("%w %s", ErrNoAccount, id)
	}
	return a.pool.Remove(ctx, id)
}

// importAccount replaces matching identities and selects an entry's first account.
func (a *Accounts) importAccount(ctx context.Context, account NewAccount, source string) (types.AccountInfo, error) {
	var existing AccountMeta
	found := false
	var err error
	if account.Label.IdentityKey != nil {
		existing, found, err = FindByIdentity(ctx, a.pool, *account.Label.IdentityKey)
		if err != nil {
			return types.AccountInfo{}, err
		}
	}
	id := CredentialIDFor(account.Label.IdentityKey, account.Label.Label)
	if found {
		id = existing.ID
	}
	meta := AccountMeta{
		ID:          id,
		Label:       account.Label.Label,
		Detail:      account.Label.Detail,
		UpdatedAt:   a.clock.Now(),
		Source:      source,
		IdentityKey: account.Label.IdentityKey,
	}
	if err := a.pool.Write(ctx, meta, account.Secret); err != nil {
		return types.AccountInfo{}, err
	}
	_, ok, err := a.pool.Active(ctx)
	if err != nil {
		return types.AccountInfo{}, err
	}
	if !ok {
		if err := a.pool.SetActive(ctx, id); err != nil {
			return types.AccountInfo{}, err
		}
	}
	return meta.Info(), nil
}

// AuthReason distinguishes missing, corrupt, inaccessible and refused credentials.
type AuthReason uint8

// Authentication failure reasons shared by provider operations.
const (
	// AuthReasonMissing indicates that the configured account has no readable credentials.
	AuthReasonMissing AuthReason = iota
	// AuthReasonInvalid indicates that the secret document cannot be decoded.
	AuthReasonInvalid
	// AuthReasonStore indicates that the credential store operation failed.
	AuthReasonStore
	// AuthReasonRefresh indicates that the token refresh failed.
	AuthReasonRefresh
)

// AuthError describes why a family's account could not be used, without tokens.
type AuthError struct {
	Family string
	Reason AuthReason
	Detail string
}

// AccountAuthFailure classifies a failed account read or renewal; an error
// that already holds an AuthError returns it unchanged.
func AccountAuthFailure(family string, err error) AuthError {
	var failure AuthError
	if errors.As(err, &failure) {
		return failure
	}
	var renewal *RenewError
	var decode *SecretDecodeError
	switch {
	case errors.As(err, &renewal):
		return AuthError{Family: family, Reason: AuthReasonRefresh, Detail: err.Error()}
	case errors.Is(err, ErrNoSecretDocument):
		return AuthError{Family: family, Reason: AuthReasonMissing, Detail: err.Error()}
	case errors.As(err, &decode):
		return AuthError{
			Family: family,
			Reason: AuthReasonInvalid,
			Detail: "its secret document is " + decode.Error(),
		}
	default:
		return AuthError{Family: family, Reason: AuthReasonStore, Detail: err.Error()}
	}
}

// Error returns the diagnostic for this failure.
func (f AuthError) Error() string {
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
func (f AuthError) Failure() Failure {
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
func (f AuthError) State() types.AuthState {
	message := f.Error()
	if f.Reason == AuthReasonMissing {
		return &types.Unauthenticated{Message: &message}
	}
	return &types.AuthError{Message: message}
}
