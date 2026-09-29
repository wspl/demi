package edge

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/netip"
	"net/url"
	"os"
	"path/filepath"
	"sync/atomic"

	whatwg "github.com/nlnwa/whatwg-url/url"

	"github.com/wspl/demi/go/backend"
	"github.com/wspl/demi/go/backend/auth"
	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/internal/fsfail"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/webapi"
)

// The data directory's databases (storage.md § Ownership and layout).
const (
	controlDatabase       = "control.sqlite"
	conversationDatabases = "conversations"
)

// Config is everything Start needs: the configured values, and the parts a
// test build replaces.
type Config struct {
	// DataDir is the data directory.
	DataDir string
	// MachinesSocket is the machine manager's Unix socket: every deployment
	// has Cloud.
	MachinesSocket string
	// Address is where the listener binds; port 0 picks a free port.
	Address string
	// Mode says who configures providers (product.md § Instance mode).
	Mode webapi.InstanceMode
	// WebDirectory is a built browser directory served beside the API; ""
	// for none.
	WebDirectory string
	// PublicURL is the URL runners connect to, which the installers name;
	// nil for the origin an installer was requested from.
	PublicURL *whatwg.Url
	// ExposeDomain is the domain expose hostnames live under; nil leaves
	// exposes unavailable (expose.md § Deployment).
	ExposeDomain *backend.ExposeDomain
	// RunnerReleases is the directory of the runner releases the installer
	// routes serve; "" answers them 503.
	RunnerReleases string
	// ObjectStore is the JSON file that puts the object store in an S3
	// bucket; "" for the data directory.
	ObjectStore string
	// ObserveObjects wraps the object store before storage uses it; nil
	// leaves it as it is. A test build counts what reaches the store.
	ObserveObjects func(storage.ObjectStore) storage.ObjectStore
	// InstanceSecret is the instance secret; nil for the one in the data
	// directory, which the first start creates.
	InstanceSecret *backend.InstanceSecret
	// AccountMail delivers email verification codes; nil answers an email
	// change mail_unavailable.
	AccountMail auth.AccountMail
	Clock       core.Clock
	// Families are the provider families entries are assembled with.
	Families backend.FamilyRegistry
	// ModelsDevURL is where the models.dev document is read.
	ModelsDevURL string
	// ClaudeReleases is the Claude Code distribution whose newest release
	// the CLI on each Cloud follows (claude-code.md § Which version).
	ClaudeReleases url.URL
	// Logins says how long a device login waits for its user, and how long
	// its result is kept.
	Logins backend.LoginTiming
	// CloudCapacity is how many Clouds may be not stopped at once.
	CloudCapacity int64
}

// NewConfig is the configuration of a backend over dataDir, listening on
// address, with the product's defaults.
func NewConfig(dataDir, address string, mode webapi.InstanceMode, machinesSocket string) Config {
	releases, err := url.Parse(backend.DefaultClaudeReleasesURL)
	if err != nil {
		// The default distribution is a URL.
		panic(err)
	}
	return Config{
		DataDir:        dataDir,
		MachinesSocket: machinesSocket,
		Address:        address,
		Mode:           mode,
		Clock:          core.SystemClock{},
		Families:       backend.BuiltinFamilies(),
		ModelsDevURL:   provider.ModelsDevDefaultURL,
		ClaudeReleases: *releases,
		Logins:         backend.DefaultLoginTiming,
		CloudCapacity:  16,
	}
}

// Server is a running backend.
type Server struct {
	services *backend.Services
	shards   *backend.Shards
	http     *http.Server
	listener net.Listener
	// closing is set when shutdown starts: a request on an open connection
	// then answers 503 backend_closing.
	closing atomic.Bool
	// served closes once the server stopped accepting.
	served chan struct{}
	// closeObjects closes the object store, once storage is done with it.
	closeObjects func()
}

// Start starts the backend. It serves once this returns: the data directory
// and the instance secret, then the databases and the object store, the
// shared services and the shards, and the listener last.
func Start(ctx context.Context, config Config) (*Server, error) {
	if err := os.MkdirAll(config.DataDir, 0o755); err != nil {
		return nil, fmt.Errorf("the data directory %s cannot be created: %w", config.DataDir, fsfail.Cause(err))
	}
	secret := config.InstanceSecret
	if secret == nil {
		loaded, err := backend.LoadInstanceSecret(config.DataDir)
		if err != nil {
			return nil, err
		}
		secret = &loaded
	}
	objects, err := openObjects(ctx, config)
	if err != nil {
		return nil, err
	}
	closeObjects := func() {
		if closer, ok := objects.(io.Closer); ok {
			// Nothing was written through the store yet.
			_ = closer.Close()
		}
	}
	observed := objects
	if config.ObserveObjects != nil {
		observed = config.ObserveObjects(objects)
	}
	control, err := storage.OpenControl(ctx, filepath.Join(config.DataDir, controlDatabase), config.Clock)
	if err != nil {
		closeObjects()
		return nil, fmt.Errorf("storage cannot be opened: %w", err)
	}
	conversations, err := storage.OpenConversationStores(filepath.Join(config.DataDir, conversationDatabases), storage.MaxWriters)
	if err != nil {
		// Opening the conversation stores left nothing open; the control
		// database has a connection.
		if failure := control.Close(); failure != nil {
			slog.Error("the control database did not close after a failed start", "error", failure)
		}
		closeObjects()
		return nil, fmt.Errorf("storage cannot be opened: %w", err)
	}
	server := &Server{served: make(chan struct{}), closeObjects: closeObjects}
	server.services, err = startServices(ctx, config, *secret, control, conversations, storage.NewBlobStores(observed, config.Clock))
	if err == nil {
		err = server.serve(ctx, config)
	}
	if err != nil {
		for _, failure := range server.closeStorage() {
			slog.Error("storage did not close after a failed start", "error", failure)
		}
		return nil, err
	}
	return server, nil
}

// startServices starts what the users share.
func startServices(ctx context.Context, config Config, secret backend.InstanceSecret, control *storage.Control, conversations *storage.ConversationStores, blobs *storage.BlobStores) (*backend.Services, error) {
	hasher, err := auth.NewPasswordHasher(ctx)
	if err != nil {
		return nil, fmt.Errorf("password hashing cannot start: %w", err)
	}
	// The shared services' own client; each shard's runtimes take their
	// own.
	client := &http.Client{}
	sync := &backend.SyncRegistry{}
	vault := backend.NewVault(control, secret.VaultKey(), config.Mode, sync)
	modelsDev := provider.NewModelsDevClient(client, config.ModelsDevURL, config.Clock)
	assembly := backend.NewProviderAssembly(vault, config.Families, backend.NewAccountQuotas(vault), backend.NewModelCatalogCache(control, config.Clock), backend.NewVendorCatalog(modelsDev), client, config.Clock)
	operations := &backend.ProviderOperations{}
	machines, _ := backend.NewMachinesClient(config.MachinesSocket)
	return &backend.Services{
		Clock:          config.Clock,
		Mode:           config.Mode,
		Control:        control,
		Conversations:  conversations,
		Blobs:          blobs,
		Hasher:         hasher,
		Sessions:       &auth.WebSessions{Control: control},
		Limiter:        auth.NewLoginLimiter(config.Clock),
		Email:          auth.NewEmailChanges(control, hasher, config.AccountMail, secret.EmailCodeKey()),
		Sync:           sync,
		Vault:          vault,
		Assembly:       assembly,
		ClaudeReleases: backend.NewClaudeReleases(config.ClaudeReleases),
		Operations:     operations,
		Logins:         backend.NewLoginFlows(assembly, operations, config.Logins),
		Machines:       machines,
		CloudCapacity:  backend.NewCloudCapacity(config.CloudCapacity),
	}, nil
}

// serve starts the shards, settles the Clouds an earlier backend left, and
// starts the listener.
func (s *Server) serve(ctx context.Context, config Config) error {
	s.shards = backend.NewShards(s.services)
	if err := backend.RecoverResets(ctx, s.services); err != nil {
		s.stopServices(ctx)
		return fmt.Errorf("the Clouds cannot be recovered: %w", err)
	}
	// An IPv4 address listens on IPv4 alone, as the Rust's does: Go takes
	// "0.0.0.0" for every address of the system, IPv6's among them.
	network := "tcp"
	if host, _, err := net.SplitHostPort(config.Address); err == nil {
		if address, err := netip.ParseAddr(host); err == nil && address.Is4() {
			network = "tcp4"
		}
	}
	listener, err := net.Listen(network, config.Address)
	if err != nil {
		s.stopServices(ctx)
		return fmt.Errorf("the backend cannot listen on %s: %w", config.Address, err)
	}
	s.listener = listener
	site := &site{publicURL: config.PublicURL}
	s.http = &http.Server{Handler: s.routes(site, config.WebDirectory), ErrorLog: slog.NewLogLogger(slog.Default().Handler(), slog.LevelDebug)}
	go func() {
		defer close(s.served)
		// Serve ends when shutdown closes the listener.
		if err := s.http.Serve(listener); err != nil && !errors.Is(err, net.ErrClosed) && !errors.Is(err, http.ErrServerClosed) {
			slog.Error("the listener stopped", "error", err)
		}
	}()
	return nil
}

// Addr is the address the listener is bound to.
func (s *Server) Addr() net.Addr { return s.listener.Addr() }

// Close shuts the backend down. The listener closes first, so no new work
// starts and a new request on an open connection answers 503
// backend_closing; every step runs even when an earlier one fails, and the
// failures are reported together.
func (s *Server) Close(ctx context.Context) error {
	s.closing.Store(true)
	// A listener that is closed already has nothing to stop.
	_ = s.listener.Close()
	<-s.served
	var failures []error
	s.services.Logins.Close()
	for _, failure := range s.shards.Close(ctx) {
		failures = append(failures, fmt.Errorf("a Cloud was not saved: %s", failure))
	}
	if err := s.services.Machines.Close(ctx); err != nil {
		failures = append(failures, fmt.Errorf("the machine manager did not reconcile: %w", err))
	}
	if err := s.http.Close(); err != nil {
		failures = append(failures, fmt.Errorf("the listener did not stop cleanly: %w", err))
	}
	s.services.Assembly.Close()
	failures = append(failures, s.closeStorage()...)
	if len(failures) > 0 {
		return fmt.Errorf("shutdown failed: %w", errors.Join(failures...))
	}
	return nil
}

// stopServices cancels the logins and drains the catalog refreshes and
// quota writes, after a start that failed once the services ran.
func (s *Server) stopServices(ctx context.Context) {
	s.services.Logins.Close()
	s.shards.Close(ctx)
	s.services.Assembly.Close()
}

// closeStorage closes the conversation databases, then the control
// database and the object store, and answers every step that failed.
func (s *Server) closeStorage() []error {
	var failures []error
	if s.services != nil {
		if err := s.services.Conversations.Close(); err != nil {
			failures = append(failures, fmt.Errorf("a conversation database did not close: %w", err))
		}
		if err := s.services.Control.Close(); err != nil {
			failures = append(failures, fmt.Errorf("the control database did not close: %w", err))
		}
	}
	s.closeObjects()
	return failures
}

// openObjects is the object store: the S3 bucket the configuration names,
// or the data directory.
func openObjects(ctx context.Context, config Config) (storage.ObjectStore, error) {
	if config.ObjectStore == "" {
		return storage.OpenLocalObjects(config.DataDir)
	}
	s3, err := storage.ReadS3Config(config.ObjectStore)
	if err != nil {
		return nil, fmt.Errorf("DEMI_OBJECT_STORE_CONFIG cannot be used: %w", err)
	}
	return storage.OpenS3Objects(ctx, s3)
}
