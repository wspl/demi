package host

import (
	"context"
	"errors"
	"io"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"strings"
	"sync"
	"syscall"

	"github.com/go-git/go-git/v5/plumbing"
	"github.com/go-git/go-git/v5/plumbing/filemode"
	"github.com/go-git/go-git/v5/plumbing/format/index"
	"github.com/go-git/go-git/v5/plumbing/object"
)

var errNotDirectory = error(syscall.ENOTDIR)

type gitSignal struct {
	index, work    byte
	conflict, from string
	untracked      bool
}

func (s gitSignal) status() string {
	if s.conflict != "" {
		return s.conflict
	}
	if s.untracked {
		return "??"
	}
	if s.index == 0 && s.work == 0 {
		return ""
	}
	a, b := s.index, s.work
	if a == 0 {
		a = ' '
	}
	if b == 0 {
		b = ' '
	}
	return string([]byte{a, b})
}

// indexSignals compares the two committed sides and delegates staged rename
// similarity to go-git, preserving all conflict stages and intent-to-add entries.
func indexSignals(ctx context.Context, tree *object.Tree, head map[string]object.TreeEntry, idx *index.Index) (map[string]gitSignal, map[string]*index.Entry, error) {
	signals := make(map[string]gitSignal)
	tracked := make(map[string]*index.Entry)
	stages := make(map[string]int)
	for _, entry := range idx.Entries {
		if err := ctx.Err(); err != nil {
			return nil, nil, err
		}
		if entry.Stage != 0 {
			stages[entry.Name] |= 1 << (entry.Stage - 1)
			continue
		}
		tracked[entry.Name] = entry
		if entry.IntentToAdd {
			continue
		}
		h, ok := head[entry.Name]
		signal := gitSignal{}
		if !ok {
			signal.index = 'A'
		} else if h.Hash != entry.Hash || h.Mode != entry.Mode {
			signal.index = 'M'
			if gitFileKind(h.Mode) != gitFileKind(entry.Mode) {
				signal.index = 'T'
			}
		}
		if signal.index != 0 {
			signals[entry.Name] = signal
		}
	}
	pairs := map[int]string{1: "DD", 2: "AU", 3: "UD", 4: "UA", 5: "DU", 6: "AA", 7: "UU"}
	for name, stages := range stages {
		signals[name] = gitSignal{conflict: pairs[stages]}
	}
	for name := range head {
		if tracked[name] == nil && stages[name] == 0 {
			signals[name] = gitSignal{index: 'D'}
		}
	}
	if tree == nil {
		return signals, tracked, nil
	}
	var changes object.Changes
	for name, signal := range signals {
		if signal.index == 'D' {
			changes = append(changes, &object.Change{From: object.ChangeEntry{Name: name, Tree: tree, TreeEntry: head[name]}})
		}
		if signal.index == 'A' {
			e := tracked[name]
			changes = append(changes, &object.Change{To: object.ChangeEntry{Name: name, Tree: tree, TreeEntry: object.TreeEntry{Name: name, Mode: e.Mode, Hash: e.Hash}}})
		}
	}
	sort.Sort(changes)
	paired, err := object.DetectRenames(changes, nil)
	if err != nil {
		return nil, nil, err
	}
	for _, change := range paired {
		if err = ctx.Err(); err != nil {
			return nil, nil, err
		}
		if change.From.Name != "" && change.To.Name != "" && change.From.Name != change.To.Name {
			signals[change.To.Name] = gitSignal{index: 'R', from: change.From.Name}
			delete(signals, change.From.Name)
		}
	}
	return signals, tracked, nil
}

func gitFileKind(mode filemode.FileMode) int {
	switch mode {
	case filemode.Symlink:
		return 1
	case filemode.Submodule:
		return 2
	default:
		return 0
	}
}

// diskStatus checks tracked Host paths in parallel; each worker owns its hash buffer.
func diskStatus(ctx context.Context, location *gitLocation, base *gitBaseline, scope []string, maxFiles int) (map[string]gitSignal, error) {
	signals := make(map[string]gitSignal)
	for name, signal := range base.indexSignals {
		if inGitScope(name, location.prefix, scope) {
			signals[name] = signal
		}
	}
	var entries []*index.Entry
	for name, entry := range base.tracked {
		if inGitScope(name, location.prefix, scope) {
			entries = append(entries, entry)
		}
	}
	jobs := make(chan *index.Entry)
	var workers sync.WaitGroup
	var mu sync.Mutex // Only the result map and first failure; never file IO.
	var failure error
	for range min(8, len(entries)) {
		workers.Go(func() {
			buffer := make([]byte, 32*1024)
			for entry := range jobs {
				code, err := trackedStatus(ctx, location.workdir, entry, base.indexTime, base.fileMode, buffer)
				mu.Lock()
				if failure == nil {
					failure = err
				}
				if code != 0 {
					signal := signals[entry.Name]
					signal.work = code
					signals[entry.Name] = signal
				}
				mu.Unlock()
			}
		})
	}
	for _, entry := range entries {
		select {
		case jobs <- entry:
		case <-ctx.Done():
		}
		if ctx.Err() != nil {
			break
		}
	}
	close(jobs)
	workers.Wait()
	if failure != nil {
		return nil, failure
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	// The untracked walk may replace a staged deletion with ??, exactly as Rust.
	if err := walkUntracked(ctx, location, base, scope, signals, maxFiles); err != nil {
		return nil, err
	}
	return signals, nil
}

// trackedStatus follows Git's index flags and verifies metadata before trusting an unchanged file.
func trackedStatus(ctx context.Context, root string, e *index.Entry, indexTime int64, fileMode bool, buffer []byte) (byte, error) {
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	if e.SkipWorktree || e.Mode == filemode.Submodule {
		return 0, nil
	}
	path := filepath.Join(root, filepath.FromSlash(e.Name))
	info, err := os.Lstat(path)
	if errors.Is(err, os.ErrNotExist) || errors.Is(err, errNotDirectory) {
		return 'D', nil
	}
	if err != nil {
		return 0, err
	}
	if e.IntentToAdd {
		return 'A', nil
	}
	mode, err := filemode.NewFromOSFileMode(info.Mode())
	if err != nil || info.IsDir() {
		return 'T', nil
	}
	if gitFileKind(mode) != gitFileKind(e.Mode) {
		return 'T', nil
	}
	if fileMode && mode != e.Mode {
		return 'M', nil
	}
	if metadataMatches(info, e, indexTime) {
		return 0, nil
	}
	hash, err := worktreeHash(ctx, path, info, buffer)
	if err != nil {
		return 0, err
	}
	if hash != e.Hash {
		return 'M', nil
	}
	return 0, nil
}

// worktreeHash hashes the complete tracked file, independently of the display size limit.
func worktreeHash(ctx context.Context, path string, info os.FileInfo, buffer []byte) (plumbing.Hash, error) {
	if info.Mode()&os.ModeSymlink != 0 {
		target, err := os.Readlink(path)
		return plumbing.ComputeHash(plumbing.BlobObject, []byte(target)), err
	}
	file, err := os.Open(path)
	if err != nil {
		return plumbing.ZeroHash, err
	}
	hasher := plumbing.NewHasher(plumbing.BlobObject, info.Size())
	_, err = io.CopyBuffer(hasher, &contextReader{ctx: ctx, reader: file}, buffer)
	return hasher.Sum(), errors.Join(err, file.Close())
}

// inGitScope selects a root's paths, including descendants of a touched directory.
func inGitScope(name, prefix string, scope []string) bool {
	relative, ok := relativeGitPath(name, prefix)
	if !ok {
		return false
	}
	if scope == nil {
		return true
	}
	return slices.ContainsFunc(scope, func(path string) bool { return relative == path || strings.HasPrefix(relative, path+"/") })
}
