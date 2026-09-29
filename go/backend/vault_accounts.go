package backend

import (
	"context"
	"errors"
	"slices"

	"github.com/google/uuid"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/webapi"
)

// setupTokenFamily is the family whose accounts are setup tokens.
const setupTokenFamily = "claude-code"

// AccountRefusalKind says which rule refused an account operation; the edge
// answers each with its own status.
type AccountRefusalKind int

const (
	// AccountExists: the owner already holds the family's subscription
	// entry.
	AccountExists AccountRefusalKind = iota + 1
	// AccountsUnsupported: the entry does not take accounts this way.
	AccountsUnsupported
	AccountNotFound
	// AccountActive: the active account cannot be removed.
	AccountActive
	// AccountTokenImportFailed: the family refused the setup token.
	AccountTokenImportFailed
)

// AccountRefusal is an account operation a rule refused (providers.md §
// Login and publication, web-api.md § Subscription accounts). A failure of
// the store or of the provider's assembly is returned as itself.
type AccountRefusal struct {
	Kind    AccountRefusalKind
	Message string
}

func (r *AccountRefusal) Error() string { return r.Message }

// accountMessages are the messages of the refusals whose message is fixed.
var accountMessages = map[AccountRefusalKind]string{
	AccountExists:   "Add this token to the existing Claude Code provider",
	AccountNotFound: "No such account",
	AccountActive:   "Select another account before removing the active one, or delete the provider",
	// A fixed message, since the family's own may quote the token.
	AccountTokenImportFailed: "The setup token could not be imported",
}

// refusedAccount is the refusal of a kind whose message is fixed.
func refusedAccount(kind AccountRefusalKind) *AccountRefusal {
	return &AccountRefusal{kind, accountMessages[kind]}
}

// ImportSetupToken creates owner's Claude Code entry from a setup token:
// the token becomes the entry's first account, published with the entry in
// one transaction.
func (a *ProviderAssembly) ImportSetupToken(ctx context.Context, owner webapi.UserID, label, token string) (ProviderEntry, error) {
	if _, ok := a.families.Get(setupTokenFamily); !ok {
		return ProviderEntry{}, &AccountRefusal{AccountsUnsupported, "Setup-token import is unavailable"}
	}
	entries, err := a.vault.Entries(ctx, owner)
	if err != nil {
		return ProviderEntry{}, err
	}
	if slices.ContainsFunc(entries, func(entry ProviderEntry) bool { return entry.Family == setupTokenFamily }) {
		return ProviderEntry{}, refusedAccount(AccountExists)
	}
	staged := provider.NewMemoryCredentialPool()
	made, err := a.Detached(setupTokenFamily, uuid.NewString(), label, staged)
	if err != nil {
		return ProviderEntry{}, err
	}
	if _, err := addAccount(ctx, made, token); err != nil {
		return ProviderEntry{}, err
	}
	entry, err := a.vault.CreateSubscription(ctx, owner, setupTokenFamily, label, staged)
	if err != nil {
		return ProviderEntry{}, err
	}
	if entry == nil {
		return ProviderEntry{}, refusedAccount(AccountExists)
	}
	return *entry, nil
}

// AddToken adds a setup token's account to entry.
func (a *ProviderAssembly) AddToken(ctx context.Context, entry ProviderEntry, token string) (core.AccountInfo, error) {
	if err := subscriptionOnly(entry); err != nil {
		return core.AccountInfo{}, err
	}
	made, err := a.ProviderFor(ctx, entry)
	if err != nil {
		return core.AccountInfo{}, err
	}
	account, err := addAccount(ctx, made, token)
	if err != nil {
		return core.AccountInfo{}, err
	}
	return account, a.Invalidate(context.WithoutCancel(ctx), entry.ID)
}

// addAccount stores the token's account through the provider's own
// account operations, which know the family's secret document.
func addAccount(ctx context.Context, made provider.Provider, token string) (core.AccountInfo, error) {
	unsupported := &AccountRefusal{AccountsUnsupported, "Use device login for this provider"}
	owner, ok := made.(provider.AccountsProvider)
	if !ok || !owner.Accounts().Capability().Add {
		return core.AccountInfo{}, unsupported
	}
	secret, err := provider.NewSecret(token)
	if err != nil {
		return core.AccountInfo{}, refusedAccount(AccountTokenImportFailed)
	}
	account, err := owner.Accounts().Add(ctx, provider.AddAccount{SetupToken: secret})
	var store *poolStoreError
	switch {
	case err == nil:
		return account, nil
	case errors.Is(err, provider.ErrAddUnsupported):
		return core.AccountInfo{}, unsupported
	case errors.As(err, &store):
		return core.AccountInfo{}, store
	default:
		// Whatever else the family says may quote the token.
		return core.AccountInfo{}, refusedAccount(AccountTokenImportFailed)
	}
}

// ListAccounts are the entry's accounts and the active one; for a user who
// only infers, disclose is false and there are none.
func (a *ProviderAssembly) ListAccounts(ctx context.Context, entry ProviderEntry, disclose bool) (webapi.Accounts, error) {
	if err := subscriptionOnly(entry); err != nil {
		return webapi.Accounts{}, err
	}
	listed := webapi.Accounts{Accounts: []core.AccountInfo{}}
	if !disclose {
		return listed, nil
	}
	records, err := a.vault.Accounts(ctx, entry.ID)
	if err != nil {
		return webapi.Accounts{}, err
	}
	for _, record := range records {
		listed.Accounts = append(listed.Accounts, accountMeta(record).Info())
	}
	listed.Active = entry.Active()
	return listed, nil
}

// ActivateAccount makes account the one the entry infers with. The next
// request builds its runtime from the account's provider; a running one
// finishes with its own.
func (a *ProviderAssembly) ActivateAccount(ctx context.Context, entry ProviderEntry, account webapi.CredentialID) (webapi.CredentialID, error) {
	if err := subscriptionOnly(entry); err != nil {
		return webapi.CredentialID{}, err
	}
	selected, err := a.vault.control.SetActiveCredential(ctx, entry.ID, account)
	if err != nil {
		return webapi.CredentialID{}, err
	}
	if !selected {
		return webapi.CredentialID{}, refusedAccount(AccountNotFound)
	}
	a.vault.markChanged(entry.Owner)
	return account, a.Invalidate(context.WithoutCancel(ctx), entry.ID)
}

// RemoveAccount removes an account other than the active one, with its
// quota snapshot.
func (a *ProviderAssembly) RemoveAccount(ctx context.Context, entry ProviderEntry, account webapi.CredentialID) error {
	if err := subscriptionOnly(entry); err != nil {
		return err
	}
	if active := entry.Active(); active != nil && *active == account {
		return refusedAccount(AccountActive)
	}
	stored, err := a.vault.Account(ctx, entry.ID, account)
	if err != nil {
		return err
	}
	if stored == nil {
		return refusedAccount(AccountNotFound)
	}
	if err := a.vault.control.RemoveCredential(ctx, entry.ID, account); err != nil {
		return err
	}
	a.quotas.ForgetAccount(entry.ID, account)
	a.vault.markChanged(entry.Owner)
	return a.Invalidate(context.WithoutCancel(ctx), entry.ID)
}

// subscriptionOnly refuses an entry that does not take subscription
// accounts.
func subscriptionOnly(entry ProviderEntry) error {
	if _, ok := entry.Credential.(Subscription); !ok {
		return &AccountRefusal{AccountsUnsupported, "This provider does not use subscription accounts"}
	}
	return nil
}
