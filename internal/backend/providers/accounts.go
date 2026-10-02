package providers

import (
	"context"
	"errors"
	"fmt"

	"github.com/google/uuid"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/webapi"
)

// SetupTokenFamily is the family whose accounts are setup tokens.
const SetupTokenFamily = "claude-code"

// ImportSetupToken creates the Claude Code entry and its first account in one transaction.
func ImportSetupToken(ctx context.Context, assembly *Assembly, owner webapi.UserID, label, token string) (ProviderEntry, error) {
	if assembly.families.Family(SetupTokenFamily) == nil {
		return ProviderEntry{}, &AccountRefusal{Kind: AccountUnsupported, Message: "Setup-token import is unavailable"}
	}
	entries, err := assembly.vault.Entries(ctx, owner)
	if err != nil {
		return ProviderEntry{}, accountAssembly(err)
	}
	for _, entry := range entries {
		if entry.Family == SetupTokenFamily {
			return ProviderEntry{}, &AccountRefusal{Kind: AccountExists}
		}
	}
	staged := provider.NewMemoryCredentialPool()
	id, err := uuid.NewRandom()
	if err != nil {
		return ProviderEntry{}, accountAssembly(fmt.Errorf("create provider identity: %w", err))
	}
	p, err := assembly.Detached(SetupTokenFamily, id.String(), label, staged)
	if err != nil {
		return ProviderEntry{}, accountAssembly(err)
	}
	if _, err := addSetupToken(ctx, p, token); err != nil {
		return ProviderEntry{}, err
	}
	e, err := assembly.vault.CreateSubscription(ctx, owner, SetupTokenFamily, label, staged)
	if err != nil {
		return ProviderEntry{}, accountAssembly(err)
	}
	if e == nil {
		return ProviderEntry{}, &AccountRefusal{Kind: AccountExists}
	}
	return *e, nil
}

// AddToken adds a setup token account to an entry.
func AddToken(ctx context.Context, assembly *Assembly, entry ProviderEntry, token string) (core.AccountInfo, error) {
	if err := subscription(entry); err != nil {
		return core.AccountInfo{}, err
	}
	p, err := assembly.ProviderFor(ctx, entry)
	if err != nil {
		return core.AccountInfo{}, accountAssembly(err)
	}
	a, err := addSetupToken(ctx, p, token)
	if err != nil {
		return core.AccountInfo{}, err
	}
	if err := assembly.Invalidate(context.WithoutCancel(ctx), entry.ID); err != nil {
		return core.AccountInfo{}, accountAssembly(err)
	}
	return a, nil
}

// ListAccounts returns accounts and active selection only when disclose is true.
func ListAccounts(ctx context.Context, assembly *Assembly, entry ProviderEntry, disclose bool) (webapi.Accounts, error) {
	if err := subscription(entry); err != nil {
		return webapi.Accounts{}, err
	}
	result := webapi.Accounts{Accounts: []core.AccountInfo{}}
	if !disclose {
		return result, nil
	}
	rows, err := assembly.vault.Accounts(ctx, entry.ID)
	if err != nil {
		return result, accountAssembly(err)
	}
	for _, row := range rows {
		result.Accounts = append(result.Accounts, AccountMeta(row).Info())
	}
	result.Active = entry.Active()
	return result, nil
}

// ActivateAccount selects the account for subsequent requests.
func ActivateAccount(ctx context.Context, assembly *Assembly, entry ProviderEntry, account webapi.CredentialID) (webapi.CredentialID, error) {
	if err := subscription(entry); err != nil {
		return "", err
	}
	ctx = context.WithoutCancel(ctx)
	ok, err := assembly.vault.control.SetActiveCredential(ctx, entry.ID, account)
	if err != nil {
		return "", accountAssembly(err)
	}
	if !ok {
		return "", &AccountRefusal{Kind: AccountNotFound}
	}
	assembly.vault.MarkChanged(entry.Owner)
	if err := assembly.Invalidate(ctx, entry.ID); err != nil {
		return "", accountAssembly(err)
	}
	return account, nil
}

// RemoveAccount removes a non-active account and its quota snapshot.
func RemoveAccount(ctx context.Context, assembly *Assembly, entry ProviderEntry, account webapi.CredentialID) error {
	if err := subscription(entry); err != nil {
		return err
	}
	if active := entry.Active(); active != nil && *active == account {
		return &AccountRefusal{Kind: AccountActive}
	}
	row, err := assembly.vault.Account(ctx, entry.ID, account)
	if err != nil {
		return accountAssembly(err)
	}
	if row == nil {
		return &AccountRefusal{Kind: AccountNotFound}
	}
	ctx = context.WithoutCancel(ctx)
	if err := assembly.vault.control.RemoveCredential(ctx, entry.ID, account); err != nil {
		return accountAssembly(err)
	}
	assembly.quotas.ForgetAccount(entry.ID, account)
	assembly.vault.MarkChanged(entry.Owner)
	if err := assembly.Invalidate(ctx, entry.ID); err != nil {
		return accountAssembly(err)
	}
	return nil
}

// addSetupToken sanitizes vendor failures before they can become a product response.
func addSetupToken(ctx context.Context, p provider.Provider, token string) (core.AccountInfo, error) {
	accounts := p.Accounts()
	if accounts == nil || !accounts.Capability().Add {
		return core.AccountInfo{}, &AccountRefusal{Kind: AccountUnsupported, Message: "Use device login for this provider"}
	}
	secret, err := provider.NewSecret(token)
	if err != nil {
		return core.AccountInfo{}, &AccountRefusal{Kind: AccountTokenImportFailed}
	}
	account, err := accounts.Add(ctx, provider.AddAccount{SetupToken: secret})
	if err == nil {
		return account, nil
	}
	if errors.Is(err, provider.ErrAccountsUnsupported) {
		return core.AccountInfo{}, &AccountRefusal{Kind: AccountUnsupported, Message: "Use device login for this provider"}
	}
	var pool *provider.PoolError
	if errors.As(err, &pool) && pool.Err != nil {
		return core.AccountInfo{}, &AccountRefusal{Kind: AccountStore, Message: pool.Error()}
	}
	return core.AccountInfo{}, &AccountRefusal{Kind: AccountTokenImportFailed}
}
func accountAssembly(err error) error {
	var assembly *AssemblyError
	if !errors.As(err, &assembly) {
		err = &AssemblyError{Kind: AssemblyStorage, Err: err}
	}
	return &AccountRefusal{Kind: AccountAssembly, Err: err}
}
func subscription(entry ProviderEntry) error {
	if _, ok := entry.Credential.(*SubscriptionCredential); !ok {
		return &AccountRefusal{Kind: AccountUnsupported, Message: "This provider does not use subscription accounts"}
	}
	return nil
}
