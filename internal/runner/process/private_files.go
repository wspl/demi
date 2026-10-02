package process

//revive:disable:unused-parameter // API checkpoint: stub parameter names document the boundary.

import "context"

// WritePrivate replaces path atomically with bytes and a final newline,
// readable by the owner alone and durable before the rename.
func WritePrivate(ctx context.Context, path string, bytes []byte) error {
	panic("not written: r-process")
}

// Chmod sets path's permission bits; Windows only checks that path exists.
func Chmod(ctx context.Context, path string, mode uint32) error { panic("not written: r-process") }
