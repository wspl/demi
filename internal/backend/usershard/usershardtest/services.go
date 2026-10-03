package usershardtest

import (
	"context"
	"crypto/rand"
	"net/url"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/internal/backend/accounts"
	"github.com/wspl/demi/internal/backend/blobs"
	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/webapi"
)

// StartServices opens temporary storage and shared services without provider
// families or a listening machine manager. Cleanup joins all workers and closes
// services and storage; start routing afterwards so its cleanup runs first.
func StartServices(t testing.TB) *usershard.Services {
	t.Helper()
	return StartServicesWithLifecycle(t, usershard.DefaultLifecycleTuning())
}

// StartServicesWithLifecycle supplies shortened idle and retention timing with
// the same ownership and cleanup as StartServices.
func StartServicesWithLifecycle(t testing.TB, lifecycle usershard.LifecycleTuning) *usershard.Services {
	t.Helper()
	cleanupCtx := context.WithoutCancel(t.Context())
	lifetime, cancel := context.WithCancel(cleanupCtx)
	t.Cleanup(cancel)
	data := t.TempDir()
	clock := core.SystemClock{}
	storage := openTestStorage(t, data, clock)
	vaultKey, codeKey := randomServiceKeys(t)
	models, err := url.Parse(provider.ModelsDevURL)
	if err != nil {
		t.Fatal(err)
	}
	releases, err := url.Parse(providers.DefaultReleasesURL)
	if err != nil {
		t.Fatal(err)
	}
	machines, _ := cloud.NewClient(lifetime, filepath.Join(data, "machines.sock"))

	services, err := usershard.StartServices(
		t.Context(),
		storage,
		usershard.ServiceKeys{
			Vault:      *providers.NewVaultKey(vaultKey),
			EmailCodes: accounts.NewCodeKey(codeKey),
		},
		usershard.ProviderSetup{
			Families:       &providers.FamilyRegistry{},
			ModelsDevURL:   models,
			ClaudeReleases: releases,
			Logins:         providers.DefaultLoginTiming(),
			Clock:          clock,
		},
		usershard.ServiceSettings{
			Mode:          webapi.InstanceModeShared,
			Runners:       usershard.DefaultRunnerTuning(),
			Conversations: usershard.DefaultConversationTuning(),
			Pages:         usershard.DefaultPageTuning(),
			Native:        runners.UnpublishedCatalog(),
			Cloud:         cloud.NewServices(machines, cloud.DefaultTuning()),
			Lifecycle:     lifecycle,
			Exposes:       usershard.DefaultExposeTuning(),
		},
	)
	if err != nil {
		if closeErr := machines.Close(context.Background()); closeErr != nil {
			t.Error(closeErr)
		}
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := services.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return services
}

// StartShards starts routing over services and registers joined test cleanup.
func StartShards(t testing.TB, services *usershard.Services) *usershard.Shards {
	t.Helper()
	shards, err := usershard.NewShards(t.Context(), services)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := shards.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return shards
}

// openTestStorage registers blob and database cleanup in their ownership order.
func openTestStorage(t testing.TB, data string, clock core.Clock) *usershard.Storage {
	objects, err := blobs.Open(t.Context(), data, nil)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := objects.Close(); err != nil {
			t.Error(err)
		}
	})
	storage, err := usershard.OpenStorage(t.Context(), data, clock, objects)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := storage.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return storage
}

// randomServiceKeys supplies independent vault and email-code keys for the service fixture.
func randomServiceKeys(t testing.TB) (vaultKey, codeKey [32]byte) {
	if _, err := rand.Read(vaultKey[:]); err != nil {
		t.Fatal(err)
	}
	if _, err := rand.Read(codeKey[:]); err != nil {
		t.Fatal(err)
	}
	return vaultKey, codeKey
}
