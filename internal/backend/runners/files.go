package runners

import (
	"context"
	"errors"
	"strings"

	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapiproto"
)

// TextRefusal says why a file is not shown as text. Use errors.Is to compare it.
type TextRefusal string

const (
	// ErrTextTooLarge means the file exceeds the edit snapshot byte limit.
	ErrTextTooLarge TextRefusal = "The file is too large to show"
	// ErrTextNotText means the file is not UTF-8 without NUL bytes.
	ErrTextNotText TextRefusal = "The file is not UTF-8 text"
)

// Error returns the refusal shown to the user.
func (r TextRefusal) Error() string {
	return string(r)
}

// TextOf interprets bytes as UTF-8 without NUL bytes, within the edit snapshot limit.
func TextOf(bytes []byte) (string, error) {
	if len(bytes) > cmdproto.EditFileBytes {
		return "", ErrTextTooLarge
	}
	if !cmdproto.IsText(bytes) {
		return "", ErrTextNotText
	}
	return string(bytes), nil
}

// ReadTextFile reads one Host file as text; an oversized file is refused before
// reading its bytes. Errors retain the host.Error or TextRefusal cause.
func ReadTextFile(ctx context.Context, fs host.FS, path string) (string, error) {
	stat, err := fs.Stat(ctx, path)
	if err != nil {
		return "", err
	}
	if stat.Size > cmdproto.EditFileBytes {
		return "", ErrTextTooLarge
	}
	bytes, err := fs.ReadFile(ctx, path)
	if err != nil {
		return "", err
	}
	return TextOf(bytes)
}

// BrowseDirectory reads entries and metadata sequentially so one listing cannot
// flood the runner queue. Entries that disappear meanwhile are omitted.
func BrowseDirectory(ctx context.Context, fs host.FS, path string) ([]webapiproto.DirectoryEntry, error) {
	names, err := fs.ReadDir(ctx, path)
	if err != nil {
		return nil, err
	}
	entries := make([]webapiproto.DirectoryEntry, 0, len(names))
	for _, entry := range names {
		stat, err := fs.Lstat(ctx, strings.TrimRight(path, "/")+"/"+entry.Name)
		if err != nil {
			var failure *host.Error
			if errors.As(err, &failure) && failure.Code == "ENOENT" {
				continue
			}
			return nil, err
		}
		entries = append(
			entries,
			webapiproto.DirectoryEntry{
				Name:           entry.Name,
				IsDirectory:    entry.Kind == host.Directory,
				IsSymbolicLink: stat.Kind == host.Symlink,
				Size:           stat.Size,
				ModifiedAt:     stat.Modified,
			},
		)
	}
	return entries, nil
}
