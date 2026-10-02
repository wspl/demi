package cmdpkgs

import (
	"context"
	"errors"
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"runtime"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

func (c *ArtifactCache) preinstalled(ctx context.Context, w Wanted) (string, error) {
	if c.image == "" || runtime.GOOS == "windows" {
		return "", nil
	}
	c.mu.Lock()
	path, checked := c.checked[w.Artifact.SHA256]
	c.mu.Unlock()
	if checked {
		return path, nil
	}
	directory := filepath.Join(c.image, w.Artifact.SHA256)
	var err error
	switch form := w.Form.(type) {
	case *commandwire.ArtifactFile:
		path, err = checkImageFile(ctx, directory, digestOf(w.Artifact))
	case *commandwire.ArtifactArchive:
		path, err = artifacts.Installed(ctx, directory, artifacts.Archive{Digest: digestOf(w.Artifact), Entry: form.Entry})
		if err == nil && path == "" {
			err = os.ErrNotExist
		}
	}
	if err != nil {
		if ctx.Err() != nil {
			return "", &RuntimeError{Kind: Cancelled, Cause: ctx.Err()}
		}
		if cmdsdk.Exhausted(err) {
			return "", err
		}
		if errors.Is(err, os.ErrNotExist) {
			return "", nil
		}
		slog.Warn("preinstalled artifact in " + directory + " not used, downloading it: " + err.Error())
		path = ""
	}
	c.mu.Lock()
	c.checked[w.Artifact.SHA256] = path
	c.mu.Unlock()
	return path, nil
}

func checkImageFile(ctx context.Context, directory string, expected artifacts.Digest) (string, error) {
	entries, err := os.ReadDir(directory)
	if err != nil {
		return "", err
	}
	if len(entries) != 1 {
		return "", errors.New("the directory does not hold exactly one file")
	}
	if !entries[0].Type().IsRegular() {
		return "", errors.New("the directory's entry is not a regular file")
	}
	path := filepath.Join(directory, entries[0].Name())
	input, err := os.Open(path)
	if err != nil {
		return "", err
	}
	err = artifacts.Copy(ctx, input, expected, io.Discard)
	return path, errors.Join(err, input.Close())
}
