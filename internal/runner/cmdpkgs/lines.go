package cmdpkgs

import (
	"context"
	"errors"
	"log/slog"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/commandwire"
)

//go:generate go run ../../../tools/contractgen .

// What the cache records of an entry, beside it: its line, its version,
// the path of its file or entry, and when this cache installed it.
// +demi:root
type lineRecord struct {
	Package     string `json:"package"`
	Name        string `json:"name"`
	Version     string `json:"version"`
	Path        string `json:"path"`
	InstalledAt uint64 `json:"installedAt"`
}

type recordedLine struct {
	digest string
	record lineRecord
}

const lineSuffix = ".line.json"

func (c *ArtifactCache) record(ctx context.Context, w Wanted, path string) error {
	destination := filepath.Join(c.root, w.Artifact.SHA256+lineSuffix)
	if _, err := os.Stat(destination); err == nil {
		return nil
	} else if !errors.Is(err, os.ErrNotExist) {
		return err
	}
	record := lineRecord{Package: w.Package, Name: w.Name, Version: w.Version, Path: path, InstalledAt: uint64(max(0, time.Now().UnixMilli()))}
	data, err := record.MarshalJSON()
	if err != nil {
		return err
	}
	return artifacts.PublishBytes(ctx, destination, data, artifacts.Publication{Mode: artifacts.Replace, Permissions: artifacts.Private, Durable: true})
}
func (c *ArtifactCache) records(ctx context.Context) ([]recordedLine, error) {
	entries, err := os.ReadDir(c.root)
	if err != nil {
		return nil, err
	}
	result := make([]recordedLine, 0)
	for _, entry := range entries {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		digest, ok := strings.CutSuffix(entry.Name(), lineSuffix)
		if !ok {
			continue
		}
		path := filepath.Join(c.root, entry.Name())
		data, err := os.ReadFile(path)
		var record lineRecord
		if err == nil {
			record, err = decodeLineRecord(data)
		}
		if err != nil {
			slog.Warn(path + ": " + err.Error())
			continue
		}
		result = append(result, recordedLine{digest, record})
	}
	return result, nil
}

// Installed lists artifacts installed from downloads or the image, newest first.
func (c *ArtifactCache) Installed(ctx context.Context, pkg, name string) ([]commandwire.InstalledArtifact, error) {
	records, err := c.records(ctx)
	if err != nil {
		return nil, err
	}
	slices.SortStableFunc(records, func(a, b recordedLine) int {
		if a.record.InstalledAt > b.record.InstalledAt {
			return -1
		}
		if a.record.InstalledAt < b.record.InstalledAt {
			return 1
		}
		return 0
	})
	result := make([]commandwire.InstalledArtifact, 0)
	for _, r := range records {
		if r.record.Package == pkg && r.record.Name == name {
			result = append(result, commandwire.InstalledArtifact{Version: r.record.Version, SHA256: r.digest, Path: r.record.Path})
		}
	}
	return result, nil
}
func (c *ArtifactCache) removeOlder(ctx context.Context, w Wanted) {
	records, err := c.records(ctx)
	if err != nil {
		slog.Warn("the artifact cache could not be listed: " + err.Error())
		return
	}
	for _, r := range records {
		if r.record.Package != w.Package || r.record.Name != w.Name || r.digest == w.Artifact.SHA256 || c.holds.held(r.digest) {
			continue
		}
		path := filepath.Join(c.root, r.digest)
		info, err := os.Lstat(path)
		switch {
		case errors.Is(err, os.ErrNotExist):
			err = nil
		case err != nil:
		case info.IsDir():
			err = os.RemoveAll(path)
		default:
			err = os.Remove(path)
		}
		if err == nil {
			err = os.Remove(path + lineSuffix)
		}
		if err != nil {
			slog.Info(r.record.Name + " " + r.record.Version + " stays in the artifact cache for now: " + err.Error())
		}
	}
}
