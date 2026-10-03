package tabs

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
)

const (
	runtimePrefix = "demi-browser-"
	profilePrefix = "demi-profile-"
	profileLock   = "demi-profile.lock"
)

type environmentDirectories struct {
	runtime string
	profile string
	lock    *os.File
}

// createDirectories stages the browser lock before publishing its profile link.
func createDirectories(bases DirectoryBases) (_ *environmentDirectories, err error) {
	path, err := os.MkdirTemp(bases.Runtime, runtimePrefix)
	if err != nil {
		return nil, err
	}
	d := &environmentDirectories{runtime: path, profile: path}
	defer func() {
		if err != nil {
			err = cdp.AfterCleanup(err, d.remove(context.Background(), nil))
		}
	}()
	d.lock, err = openEnvironmentLock(filepath.Join(path, profileLock+".new"), true)
	if err != nil {
		return nil, err
	}
	held, err := lockEnvironment(d.lock)
	if err != nil {
		return nil, err
	}
	if !held {
		return nil, errors.New("a new browser environment is already locked")
	}
	if err = os.Rename(filepath.Join(path, profileLock+".new"), filepath.Join(path, profileLock)); err != nil {
		return nil, err
	}
	if runtime.GOOS != "windows" {
		profile := filepath.Join(bases.Profiles, profilePrefix+strings.TrimPrefix(filepath.Base(path), runtimePrefix))
		if err = os.Symlink(profile, filepath.Join(path, "profile")); err != nil {
			return nil, err
		}
		if err = os.Mkdir(profile, 0o700); err != nil {
			return nil, err
		}
		d.profile = profile
	}
	return d, nil
}

// remove retires browser storage only after the process tree stopped writing it.
func (d *environmentDirectories) remove(ctx context.Context, retired error) error {
	result := retired
	if result == nil && d.profile != d.runtime {
		result = removeDirectory(ctx, d.profile)
	}
	if result == nil {
		result = removeDirectory(ctx, d.runtime)
	}
	if d.lock != nil {
		result = errors.Join(result, d.lock.Close())
		d.lock = nil
	}
	if result != nil {
		return &cdp.BrowserError{Kind: cdp.KindProfileRetained, Path: d.profile, Cause: result}
	}
	return nil
}

// removeDirectory retries Chrome's late directory writes only after retirement.
func removeDirectory(ctx context.Context, path string) error {
	deadline := time.Now().Add(300 * time.Millisecond)
	for {
		err := os.RemoveAll(path)
		if err == nil || !directoryNotEmpty(err) || !time.Now().Before(deadline) {
			return err
		}
		timer := time.NewTimer(20 * time.Millisecond)
		select {
		case <-ctx.Done():
			timer.Stop()
			return ctx.Err()
		case <-timer.C:
			timer.Stop()
		}
	}
}

// sweepOrphans examines only browser directories owned by this Host user.
func sweepOrphans(ctx context.Context, bases DirectoryBases, roots []string, owner uint32) error {
	entries, err := os.ReadDir(bases.Runtime)
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		slog.Warn("could not list browser runtime directories", "error", err)
	}
	for _, entry := range entries {
		if err := ctx.Err(); err != nil {
			return err
		}
		path := filepath.Join(bases.Runtime, entry.Name())
		if !strings.HasPrefix(entry.Name(), runtimePrefix) || !ownsDirectory(path, owner) {
			continue
		}
		if err := sweepOrphan(ctx, path, roots, owner); err != nil {
			slog.Warn("could not sweep browser environment", "path", path, "error", err)
		}
	}
	if runtime.GOOS == "windows" {
		return nil
	}
	entries, err = os.ReadDir(bases.Profiles)
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		slog.Warn("could not list browser profiles", "error", err)
	}
	for _, entry := range entries {
		if err := ctx.Err(); err != nil {
			return err
		}
		path := filepath.Join(bases.Profiles, entry.Name())
		if !strings.HasPrefix(entry.Name(), profilePrefix) || !ownsDirectory(path, owner) {
			continue
		}
		runtimePath := filepath.Join(bases.Runtime, runtimePrefix+strings.TrimPrefix(entry.Name(), profilePrefix))
		if _, err := os.Lstat(runtimePath); !errors.Is(err, os.ErrNotExist) {
			continue
		}
		if err := removeDirectory(ctx, path); err != nil {
			slog.Warn("could not remove orphan browser profile", "path", path, "error", err)
		}
	}
	return nil
}

// sweepOrphan holds an abandoned environment's lock through process and disk cleanup.
func sweepOrphan(ctx context.Context, path string, roots []string, owner uint32) (err error) {
	lock, err := openEnvironmentLock(filepath.Join(path, profileLock), false)
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, lock.Close()) }()
	held, err := lockEnvironment(lock)
	if err != nil || !held {
		return err
	}
	process := chromeProcess{runtime: path, installations: roots}
	if err := process.terminate(ctx); err != nil {
		return err
	}
	if runtime.GOOS != "windows" {
		profile, err := os.Readlink(filepath.Join(path, "profile"))
		if err != nil && !errors.Is(err, os.ErrNotExist) {
			return err
		}
		if err == nil && strings.HasPrefix(filepath.Base(profile), profilePrefix) && ownsDirectory(profile, owner) {
			if err := removeDirectory(ctx, profile); err != nil {
				return fmt.Errorf("remove browser profile: %w", err)
			}
		}
	}
	return removeDirectory(ctx, path)
}
