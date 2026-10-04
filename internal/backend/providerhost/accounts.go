package providerhost

import (
	"context"
	"errors"
	"fmt"

	"github.com/google/uuid"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// SetupTokenFamily is the family whose accounts are setup tokens.
const SetupTokenFamily = "claude-code"

// ImportSetupToken creates the Claude Code entry and its first account in one transaction.
func ImportSetupToken(
	ctx context.Context,
	assembly *Assembly,
	owner webapiproto.UserID,
	label, token string,
) (Entry, error) {
	if assembly.families.Family(SetupTokenFamily) == nil {
		return Entry{}, ErrSetupTokenUnavailable
	}
	entries, err := assembly.vault.Entries(ctx, owner)
	if err != nil {
		return Entry{}, err
	}
	for _, entry := range entries {
		if entry.Family == SetupTokenFamily {
			return Entry{}, ErrSetupTokenProviderExists
		}
	}
	staged := provider.NewMemoryCredentialPool()
	id, err := uuid.NewRandom()
	if err != nil {
		return Entry{}, fmt.Errorf("create provider identity: %w", err)
	}
	p, err := assembly.Detached(SetupTokenFamily, id.String(), label, staged)
	if err != nil {
		return Entry{}, err
	}
	if _, err := addSetupToken(ctx, p, token); err != nil {
		return Entry{}, err
	}
	e, err := assembly.vault.CreateSubscription(ctx, owner, SetupTokenFamily, label, staged)
	if errors.Is(err, database.ErrSubscriptionExists) {
		return Entry{}, ErrSetupTokenProviderExists
	}
	if err != nil {
		return Entry{}, err
	}
	return e, nil
}

// AddToken adds a setup token account to an entry.
func AddToken(
	ctx context.Context,
	assembly *Assembly,
	entry Entry,
	token string,
) (types.AccountInfo, error) {
	if err := subscription(entry); err != nil {
		return types.AccountInfo{}, err
	}
	p, err := assembly.ProviderFor(ctx, entry)
	if err != nil {
		return types.AccountInfo{}, err
	}
	a, err := addSetupToken(ctx, p, token)
	if err != nil {
		return types.AccountInfo{}, err
	}
	if err := assembly.Invalidate(context.WithoutCancel(ctx), entry.ID); err != nil {
		return types.AccountInfo{}, err
	}
	return a, nil
}

// ListAccounts returns accounts and active selection only when disclose is true.
func ListAccounts(
	ctx context.Context,
	assembly *Assembly,
	entry Entry,
	disclose bool,
) (webapiproto.Accounts, error) {
	if err := subscription(entry); err != nil {
		return webapiproto.Accounts{}, err
	}
	result := webapiproto.Accounts{Accounts: []types.AccountInfo{}}
	if !disclose {
		return result, nil
	}
	rows, err := assembly.vault.Accounts(ctx, entry.ID)
	if err != nil {
		return result, err
	}
	for _, row := range rows {
		meta := AccountMeta(row)
		result.Accounts = append(result.Accounts, meta.Info())
	}
	result.Active = entry.Active()
	return result, nil
}

// ActivateAccount selects the account for subsequent requests.
func ActivateAccount(
	ctx context.Context,
	assembly *Assembly,
	entry Entry,
	account webapiproto.CredentialID,
) (webapiproto.CredentialID, error) {
	if err := subscription(entry); err != nil {
		return "", err
	}
	ctx = context.WithoutCancel(ctx)
	ok, err := assembly.vault.control.SetActiveCredential(ctx, entry.ID, account)
	if err != nil {
		return "", err
	}
	if !ok {
		return "", ErrAccountNotFound
	}
	assembly.vault.MarkChanged(entry.Owner)
	if err := assembly.Invalidate(ctx, entry.ID); err != nil {
		return "", err
	}
	return account, nil
}

// RemoveAccount removes a non-active account and its quota snapshot.
func RemoveAccount(
	ctx context.Context,
	assembly *Assembly,
	entry Entry,
	account webapiproto.CredentialID,
) error {
	if err := subscription(entry); err != nil {
		return err
	}
	if active := entry.Active(); active != nil && *active == account {
		return ErrActiveAccount
	}
	_, found, err := assembly.vault.Account(ctx, entry.ID, account)
	if err != nil {
		return err
	}
	if !found {
		return ErrAccountNotFound
	}
	ctx = context.WithoutCancel(ctx)
	if err := assembly.vault.control.RemoveCredential(ctx, entry.ID, account); err != nil {
		return err
	}
	assembly.quotas.ForgetAccount(entry.ID, account)
	assembly.vault.MarkChanged(entry.Owner)
	if err := assembly.Invalidate(ctx, entry.ID); err != nil {
		return err
	}
	return nil
}

// addSetupToken sanitizes vendor failures before they can become a product response.
func addSetupToken(ctx context.Context, p provider.Provider, token string) (types.AccountInfo, error) {
	accounts := p.Accounts()
	if accounts == nil || !accounts.Capability().Add {
		return types.AccountInfo{}, ErrUseDeviceLogin
	}
	secret, err := provider.NewSecret(token)
	if err != nil {
		return types.AccountInfo{}, ErrTokenImportFailed
	}
	account, err := accounts.Add(ctx, provider.AddAccount{SetupToken: secret})
	if err == nil {
		return account, nil
	}
	if errors.Is(err, provider.ErrAccountsUnsupported) {
		return types.AccountInfo{}, ErrUseDeviceLogin
	}
	var pool *provider.PoolError
	if errors.As(err, &pool) {
		return types.AccountInfo{}, errors.New(pool.Error())
	}
	return types.AccountInfo{}, ErrTokenImportFailed
}

func subscription(entry Entry) error {
	if _, ok := entry.Credential.(*SubscriptionCredential); !ok {
		return ErrNotSubscription
	}
	return nil
}
