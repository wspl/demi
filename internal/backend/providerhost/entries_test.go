package providerhost

import (
	"context"
	"errors"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

func TestVaultScopeAndSecretDisclosure(t *testing.T) {
	ctx := t.Context()
	vault, owner := vaultFixture(t)
	secret := "sk-private<&\u2028-token"
	entry, err := vault.CreateAPIKey(ctx, owner.ID, "openai", "Work", APIKeyConfig{APIKey: provider.Secret(secret)})
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := contract.EncodeJSON(entry.DTO())
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(encoded), secret) || strings.Contains(string(encoded), "apiKey") {
		t.Fatal("credential disclosed in DTO")
	}
	rows, err := vault.Entries(ctx, owner.ID)
	if err != nil || len(rows) != 1 {
		t.Fatalf("entries: %v %v", rows, err)
	}
	if foreign, err := vault.Visible(ctx, "another", entry.ID); err != nil || foreign != nil {
		t.Fatalf("scope escaped: %v %v", foreign, err)
	}
	shared := NewVault(vault.control, vault.key, webapiproto.InstanceModeShared, &pagesync.Registry{})
	visible, err := shared.Visible(ctx, "another", entry.ID)
	if err != nil || visible == nil {
		t.Fatalf("shared entry missing: %v %v", visible, err)
	}
	if shared.Configures(webapiproto.UserDTO{Role: webapiproto.RoleUser}) || !shared.Configures(owner) ||
		!vault.Configures(webapiproto.UserDTO{Role: webapiproto.RoleUser}) {
		t.Fatal("scope configuration admission")
	}
	label := "Renamed"
	changed, err := vault.Update(ctx, entry.ID, &label, nil)
	if err != nil || changed.Label != label {
		t.Fatalf("update: %v %v", changed, err)
	}
	// A malformed sealed configuration is never repaired, and failure text never
	// quotes a secret even if it occurs as an unknown object key.
	for _, plain := range []string{
		`{"apiKey":123,"private":"` + secret + `"}`,
		`{"apiKey":"safe","` + secret + `":true}`,
		`{"apiKey":"safe","wireApi":"` + secret + `"}`,
		`{"apiKey":"safe","models":[{"` + secret + `":true}]}`,
	} {
		sealed, err := vault.key.Seal(ConfigRow{Provider: entry.ID}, []byte(plain))
		if err != nil {
			t.Fatal(err)
		}
		if _, err := vault.control.UpdateProvider(ctx, entry.ID, nil, &sealed); err != nil {
			t.Fatal(err)
		}
		_, err = vault.Entry(ctx, entry.ID)
		if !errors.Is(err, database.ErrCorrupt) {
			t.Fatalf("corrupt configuration accepted: %v", err)
		}
		if strings.Contains(err.Error(), secret) {
			t.Fatalf("secret escaped in error: %s", err)
		}
	}
}

func TestCatalogRestartsFromSQLiteAndRefusesCorruptStoredRecord(t *testing.T) {
	vault, owner := vaultFixture(t)
	ctx := t.Context()
	entry, err := vault.CreateAPIKey(ctx, owner.ID, "openai", "Work", APIKeyConfig{APIKey: "secret"})
	if err != nil {
		t.Fatal(err)
	}
	cache := NewModelCatalogCache(vault.control, types.SystemClock{})
	fetch := func(context.Context) (types.ProviderModelList, error) {
		return catalog("Stored"), nil
	}
	if _, err := cache.Read(ctx, entry.ID, "key", fetch, false); err != nil {
		t.Fatal(err)
	}
	if err := cache.Close(ctx); err != nil {
		t.Fatal(err)
	}
	cache = NewModelCatalogCache(vault.control, types.SystemClock{})
	defer func() {
		if err := cache.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	never := func(context.Context) (types.ProviderModelList, error) {
		t.Error("fresh record fetched")
		return types.ProviderModelList{}, errors.New("unexpected fetch")
	}
	if got, err := cache.Read(ctx, entry.ID, "key", never, false); err != nil || catalogName(got) != "Stored" {
		t.Fatalf("restart: %v %v", got, err)
	}
	if err := cache.Close(ctx); err != nil {
		t.Fatal(err)
	}
	databasetest.Execute(
		ctx,
		t,
		vault.control,
		"UPDATE model_catalogs SET record = ? WHERE provider_id = ?",
		`{"broken":true}`,
		string(entry.ID),
	)
	cache = NewModelCatalogCache(vault.control, types.SystemClock{})
	defer func() {
		if err := cache.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	if _, err := cache.Read(ctx, entry.ID, "key", never, false); err == nil {
		t.Fatal("corrupt stored catalog silently dropped")
	}
	if err := vault.Delete(ctx, entry); err != nil {
		t.Fatal(err)
	}
	if record, found, err := vault.control.CatalogRecord(ctx, entry.ID); err != nil || found {
		t.Fatalf("deleted catalog survived: %v %v", record, err)
	}
}
