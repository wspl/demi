//go:build unix

package interp_test

import (
	"context"
	"io"
	"os"
	"path/filepath"
	"sort"
	"sync"

	"mvdan.cc/sh/v3/interp"
)

// Recorder records successful write-capable opens of regular files, not reads,
// devices, FIFOs, or files an external utility opens for itself. It deliberately
// does not implement production mutation snapshots or the shared edit lock.
type demiRecorder struct {
	mu    sync.Mutex
	paths map[string]bool
	files []io.Closer
}

func (r *demiRecorder) Open(ctx context.Context, path string, flags int, mode os.FileMode) (io.ReadWriteCloser, error) {
	f, err := interp.DefaultOpenHandler()(ctx, path, flags, mode)
	if err != nil {
		return nil, err
	}
	// Retain handles so exec's persistent redirections are closed after job join.
	r.mu.Lock()
	defer r.mu.Unlock()
	r.files = append(r.files, f)
	if flags&(os.O_WRONLY|os.O_RDWR) != 0 {
		absolute := path
		if !filepath.IsAbs(absolute) {
			absolute = filepath.Join(interp.HandlerCtx(ctx).Dir, path)
		}
		info, err := os.Stat(absolute)
		if err != nil {
			f.Close()
			return nil, err
		}
		if info.Mode().IsRegular() {
			if r.paths == nil {
				r.paths = make(map[string]bool)
			}
			r.paths[filepath.Clean(absolute)] = true
		}
	}
	return f, nil
}
func (r *demiRecorder) Paths() []string {
	r.mu.Lock()
	defer r.mu.Unlock()
	paths := make([]string, 0, len(r.paths))
	for path := range r.paths {
		paths = append(paths, path)
	}
	sort.Strings(paths)
	return paths
}
func (r *demiRecorder) Close() {
	r.mu.Lock()
	defer r.mu.Unlock()
	for _, f := range r.files {
		// Most handles have already been closed by the interpreter. Closing the
		// same os.File again is safe and cannot close a reused numeric descriptor.
		f.Close()
	}
	r.files = nil
}
