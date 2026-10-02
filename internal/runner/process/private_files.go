package process

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"runtime"

	"github.com/wspl/demi/internal/artifacts"
)

// WritePrivate replaces path atomically with bytes and a final newline,
// readable by the owner alone and durable before the rename.
func WritePrivate(ctx context.Context, path string, data []byte) error {
	if !bytes.HasSuffix(data, []byte{'\n'}) {
		data = append(append([]byte(nil), data...), '\n')
	}
	if err := artifacts.PublishBytes(ctx, path, data, artifacts.Publication{Mode: artifacts.Replace, Permissions: artifacts.Private, Durable: true}); err != nil {
		return fmt.Errorf("write private file: %w", err)
	}
	return nil
}

// Chmod sets path's permission bits; Windows only checks that path exists.
func Chmod(ctx context.Context, path string, mode uint32) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if runtime.GOOS == "windows" {
		_, err := os.Stat(path)
		return err
	}
	return os.Chmod(path, os.FileMode(mode&0777)|specialMode(mode))
}

// specialMode converts the Unix special permission bits for a private directory.
func specialMode(mode uint32) os.FileMode {
	var result os.FileMode
	if mode&04000 != 0 {
		result |= os.ModeSetuid
	}
	if mode&02000 != 0 {
		result |= os.ModeSetgid
	}
	if mode&01000 != 0 {
		result |= os.ModeSticky
	}
	return result
}
