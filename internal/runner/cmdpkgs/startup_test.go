package cmdpkgs_test

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runner/cmdpkgs/cmdpkgstest"
)

// startupPeer is a deliberately mismatched service. The SDK always handles
// shutdown, so this scripted peer uses net/http directly to ignore that request.
// Its listener supplies the child stdio connection to the HTTP/2 server.
type startupPeer struct {
	conn         net.Conn
	mode         string
	marker       string
	accepted     bool
	closed       chan struct{}
	shutdown     chan struct{}
	closeOnce    sync.Once
	shutdownOnce sync.Once
}

func (p *startupPeer) Accept() (net.Conn, error) {
	if !p.accepted {
		p.accepted = true
		return p.conn, nil
	}
	<-p.closed
	return nil, net.ErrClosed
}

func (p *startupPeer) Close() error {
	p.closeOnce.Do(func() { close(p.closed) })
	return nil
}
func (p *startupPeer) Addr() net.Addr { return p.conn.LocalAddr() }
func (p *startupPeer) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	switch r.URL.Path {
	case commandwire.InfoPath:
		data, err := (commandwire.ServiceInfo{ProtocolVersion: 1, Operations: []string{"actual"}}).MarshalJSON()
		if err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		if _, err = w.Write(data); err != nil {
			return
		}
	case commandwire.ShutdownPath:
		if err := os.WriteFile(p.marker, []byte("requested"), 0o600); err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		if p.mode == "ignore" {
			<-r.Context().Done()
			return
		}
		w.WriteHeader(http.StatusOK)
		if err := http.NewResponseController(w).Flush(); err != nil {
			return
		}
		p.shutdownOnce.Do(func() { close(p.shutdown) })
	default:
		http.NotFound(w, r)
	}
}

func serveStartupPeer(mode string) error {
	peer := &startupPeer{
		conn:     &cmdsdk.PipeConn{Reader: os.Stdin, Writer: os.Stdout},
		mode:     mode,
		marker:   os.Getenv("DEMI_STARTUP_MARKER"),
		closed:   make(chan struct{}),
		shutdown: make(chan struct{}),
	}
	protocols := new(http.Protocols)
	protocols.SetUnencryptedHTTP2(true)
	server := &http.Server{Handler: peer, Protocols: protocols}
	done := make(chan error, 1)
	go func() { done <- server.Serve(peer) }()
	<-peer.shutdown
	err := server.Shutdown(context.Background())
	serveErr := <-done
	if !errors.Is(serveErr, http.ErrServerClosed) {
		err = errors.Join(err, serveErr)
	}
	if err != nil {
		return err
	}
	return os.WriteFile(peer.marker, []byte("released"), 0o600)
}

// A real unresponsive child costs six seconds: this checks the actual process
// deadline and reaping, not a timer in isolation. The cooperative case exits as
// soon as the shutdown request is answered. Neither case sleeps or polls time.
func TestStartupFailureRequestsShutdownBeforeTermination(t *testing.T) {
	executable, err := os.Executable()
	must(t, err)
	bytes, err := artifacts.DigestFile(t.Context(), executable, ^uint64(0))
	must(t, err)
	target, err := commandwire.HostTarget()
	must(t, err)
	descriptor := commandwire.PackageDescriptor{
		ID:              "demi.startup",
		Version:         "1",
		ProtocolVersion: 1,
		Operations:      []string{"expected"},
		Targets: map[string]commandwire.PackageArtifact{
			string(target): {SHA256: bytes.SHA256, Size: bytes.Size},
		},
	}
	for _, mode := range []string{"cooperative", "ignore"} {
		t.Run(mode, func(t *testing.T) {
			root := t.TempDir()
			marker := filepath.Join(root, "shutdown")
			r, err := cmdpkgs.NewServiceRegistry(
				t.Context(),
				filepath.Join(root, "cache"),
				"",
				root,
				map[string]string{"DEMI_STARTUP_PEER": mode, "DEMI_STARTUP_MARKER": marker},
			)
			must(t, err)
			defer func() { must(t, r.Close(context.Background())) }()
			start := time.Now()
			_, err = r.Handle().
				Acquire(t.Context(), descriptor, &localResolver{path: executable}, cmdpkgstest.NoNumbers{})
			elapsed := time.Since(start)
			var failure *cmdpkgs.RuntimeError
			if !errors.As(err, &failure) || failure.Kind != cmdpkgs.CatalogMismatch {
				t.Fatalf("startup failure: %v", err)
			}
			data, err := os.ReadFile(marker)
			must(t, err)
			want := "released"
			if mode == "ignore" {
				want = "requested"
				if elapsed < 6*time.Second {
					t.Fatalf("terminated before shutdown deadline: %s", elapsed)
				}
			}
			if string(data) != want {
				t.Fatalf("shutdown=%q; want %q", data, want)
			}
		})
	}
}

func startupPeerMain() {
	if err := serveStartupPeer(os.Getenv("DEMI_STARTUP_PEER")); err != nil {
		_, _ = fmt.Fprintln(os.Stderr, err) // The exit status still reports a failed fixture.
		os.Exit(1)
	}
	os.Exit(0)
}
