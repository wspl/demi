package artifactstest

import (
	"archive/zip"
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/wspl/demi/internal/artifacts"
)

// Answer describes one HTTP fixture. Gate holds the answer until an event;
// request cancellation and server cleanup also release a waiting handler.
type Answer struct {
	Status    int
	Body      []byte
	Length    bool
	Coding    string
	Gate      <-chan struct{}
	Requested chan<- struct{}
}

// OK constructs a successful fixture with its Content-Length.
func OK(body []byte) Answer { return Answer{Status: http.StatusOK, Body: body, Length: true} }

// Server owns a local fixture server and counts requests. Start registers cleanup.
type Server struct {
	server   *httptest.Server
	requests atomic.Uint64
}

// Start serves fixtures on 127.0.0.1 with automatic cancellation and cleanup.
func Start(t testing.TB, answers map[string]Answer) *Server {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	s := &Server{}
	fixtures := make(map[string]Answer, len(answers))
	for path, answer := range answers {
		answer.Body = bytes.Clone(answer.Body)
		fixtures[path] = answer
	}
	s.server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		s.requests.Add(1)
		answer, ok := fixtures[r.URL.Path]
		if !ok {
			http.NotFound(w, r)
			return
		}
		if answer.Requested != nil {
			select {
			case answer.Requested <- struct{}{}:
			case <-r.Context().Done():
				return
			case <-ctx.Done():
				return
			}
		}
		if answer.Gate != nil {
			select {
			case <-answer.Gate:
			case <-r.Context().Done():
				return
			case <-ctx.Done():
				return
			}
		}
		if answer.Length {
			w.Header().Set("Content-Length", fmt.Sprint(len(answer.Body)))
		}
		if answer.Coding != "" {
			w.Header().Set("Content-Encoding", answer.Coding)
		}
		w.WriteHeader(answer.Status)
		if !answer.Length {
			if err := http.NewResponseController(w).Flush(); err != nil {
				return
			}
		}
		_, _ = w.Write(answer.Body) // A cancelling downloader may have closed its connection.
	}))
	t.Cleanup(func() {
		cancel()
		s.server.Close()
	})
	return s
}

// URL returns a fixture's local address.
func (s *Server) URL(path string) string { return s.server.URL + path }

// Requests counts requests received by this server.
func (s *Server) Requests() uint64 { return s.requests.Load() }

// LockWaits counts installation acquisitions that found a held lock.
func LockWaits() uint64 { return artifacts.InstallLockWaits() }

// Zip constructs an archive for a fixture; entries may deliberately be invalid.
func Zip(t testing.TB, entries map[string][]byte) []byte {
	t.Helper()
	var output bytes.Buffer
	writer := zip.NewWriter(&output)
	for name, data := range entries {
		entry, err := writer.Create(name)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := entry.Write(data); err != nil {
			t.Fatal(err)
		}
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	return output.Bytes()
}

// InstallUnpacked installs a fixture release through the production installer.
// Files are hard links when permitted and copies otherwise; symlinks are
// preserved on Unix. Receipt generation and publication use the same owner
// as archive installation.
func InstallUnpacked(ctx context.Context, root string, archive artifacts.Archive, entry string) (result string, err error) {
	source := filepath.Clean(entry)
	for range strings.Split(archive.Entry, "/") {
		source = filepath.Dir(source)
	}
	if filepath.Join(source, filepath.FromSlash(archive.Entry)) != filepath.Clean(entry) {
		return "", fmt.Errorf("%s is not the archive entry %s", entry, archive.Entry)
	}
	installed, unpacking, err := artifacts.InstallArchive(ctx, root, archive)
	if err != nil || unpacking == nil {
		return installed, err
	}
	defer func() { err = errors.Join(err, unpacking.Close()) }()
	temporary, err := os.MkdirTemp(root, ".install-unpacked-")
	if err != nil {
		return "", err
	}
	defer func() { err = errors.Join(err, os.RemoveAll(temporary)) }()
	extracted := filepath.Join(temporary, "extracted")
	if err := linkArtifactTree(ctx, source, extracted); err != nil {
		return "", err
	}
	return unpacking.Publish(ctx, extracted)
}

// linkArtifactTree links an unpacked release into a private installation stage.
func linkArtifactTree(ctx context.Context, source, destination string) error {
	return filepath.WalkDir(source, func(name string, item fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if err := ctx.Err(); err != nil {
			return err
		}
		relative, err := filepath.Rel(source, name)
		if err != nil {
			return err
		}
		target := filepath.Join(destination, relative)
		if item.IsDir() {
			return os.Mkdir(target, 0755)
		}
		if item.Type()&os.ModeSymlink != 0 {
			if runtime.GOOS == "windows" {
				return fmt.Errorf("%s is a symbolic link", name)
			}
			link, err := os.Readlink(name)
			if err != nil {
				return err
			}
			return os.Symlink(link, target)
		}
		if err := os.Link(name, target); err == nil {
			return nil
		}
		// Cross-volume links and Linux protected_hardlinks can refuse an otherwise
		// readable artifact. Copying preserves the file when linking is unavailable.
		return copyArtifactFile(ctx, name, target)
	})
}

// copyArtifactFile preserves a fixture artifact's bytes and executable mode.
func copyArtifactFile(ctx context.Context, source, destination string) (err error) {
	if err := ctx.Err(); err != nil {
		return err
	}
	input, err := os.Open(source)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, input.Close()) }()
	info, err := input.Stat()
	if err != nil {
		return err
	}
	output, err := os.OpenFile(destination, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, output.Close()) }()
	if _, err := io.Copy(output, input); err != nil {
		return err
	}
	return output.Chmod(info.Mode().Perm())
}
