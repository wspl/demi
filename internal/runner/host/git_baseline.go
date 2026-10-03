package host

import (
	"context"
	"errors"
	"maps"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"time"

	git "github.com/go-git/go-git/v5"
	"github.com/go-git/go-git/v5/plumbing/format/gitignore"
	"github.com/go-git/go-git/v5/plumbing/format/index"
	"github.com/go-git/go-git/v5/plumbing/object"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/runnerwire"
)

type gitBaseline struct {
	head         *string
	entries      map[string]object.TreeEntry
	tracked      map[string]*index.Entry
	indexSignals map[string]gitSignal
	indexTime    int64
	fileMode     bool
	patterns     []gitignore.Pattern
	files        map[string]runnerwire.GitChange
	rules        []ruleStamp
	truncated    bool
}
type ruleStamp struct {
	size     int64
	modified time.Time
	present  bool
}

type treeState struct {
	baseline *gitBaseline
	// watchMu protects only notification state and startup publication.
	watchMu    sync.Mutex
	touched    map[string]bool
	whole      bool
	broken     bool
	watch      *Watch
	watchPhase string
	watchDone  chan struct{}
	cancel     context.CancelFunc
	ctx        context.Context
}

// record coalesces watcher notifications into bounded invalidation state.
func (s *treeState) record(location *gitLocation, event WatchEvent) {
	s.watchMu.Lock()
	defer s.watchMu.Unlock()
	switch event.Kind {
	case WatchLost:
		s.whole = true
	case WatchFailed:
		s.broken = true
	case WatchChanged:
		if pathWithin(event.Path, location.gitDir) || pathWithin(event.Path, location.common) {
			if !event.Metadata {
				s.whole = true
			}
		} else if !event.Metadata &&
			(filepath.Base(event.Path) == ".gitignore" || filepath.Base(event.Path) == ".gitattributes") {
			s.whole = true
		} else if !s.whole {
			s.touched[event.Path] = true
			if len(s.touched) > 100 {
				s.whole = true
				clear(s.touched)
			}
		}
	}
}

// start launches native watch startup away from the first request's computation.
func (s *treeState) start(location *gitLocation) {
	s.watchMu.Lock()
	if s.watchPhase != "" {
		s.watchMu.Unlock()
		return
	}
	s.watchPhase = "starting"
	s.watchDone = make(chan struct{})
	s.watchMu.Unlock()
	go func() {
		defer close(s.watchDone)
		trees := []string{location.root}
		for _, path := range []string{location.gitDir, location.common} {
			if !slices.ContainsFunc(trees, func(root string) bool { return pathWithin(path, root) }) {
				trees = append(trees, path)
			}
		}
		watch, err := StartWatch(s.ctx, trees, func(event WatchEvent) { s.record(location, event) })
		s.watchMu.Lock()
		if err != nil {
			s.watchPhase = "unavailable"
		} else {
			s.watchPhase = "running"
			s.watch = watch
			s.whole = true
		}
		s.watchMu.Unlock()
	}()
}

// stop joins startup before releasing a watch, even when startup outlives its request.
func (s *treeState) stop() {
	s.cancel()
	if s.watchDone != nil {
		<-s.watchDone
	}
	if s.watch != nil {
		_ = s.watch.Close(context.Background()) // A live cleanup context joins unconditionally.
	}
}

func aboveRules(location *gitLocation) []ruleStamp {
	var stamps []ruleStamp
	for path := filepath.Dir(location.root); pathWithin(path, location.workDir); path = filepath.Dir(path) {
		for _, name := range []string{".gitignore", ".gitattributes"} {
			stamp := ruleStamp{}
			if info, err := os.Stat(filepath.Join(path, name)); err == nil {
				stamp = ruleStamp{size: info.Size(), modified: info.ModTime(), present: true}
			}
			stamps = append(stamps, stamp)
		}
		if path == location.workDir {
			break
		}
	}
	return stamps
}

// computeChanges reuses a watched baseline only after adopting its startup gap,
// checking ancestor rules, and consuming all outstanding invalidation signals.
func computeChanges(
	ctx context.Context,
	state *treeState,
	root string,
	slots chan struct{},
	maxFiles int,
) (result runnerwire.GitChanges, err error) {
	defer func() {
		if recover() != nil {
			err = &gitError{code: "internal", message: "working-tree work panicked"}
		}
		if err != nil {
			state.watchMu.Lock()
			state.whole = true
			state.watchMu.Unlock()
		}
	}()
	repo, location, err := openRepository(root)
	if err != nil {
		return result, err
	}
	if location == nil {
		state.reset(ctx)
		return runnerwire.GitChanges{Files: []runnerwire.GitChange{}}, nil
	}
	defer func() { err = errors.Join(err, closeRepository(repo)) }()
	state.stopBrokenWatch()
	state.start(location)
	rules := aboveRules(location)
	scope, watched := state.changeScope(location, rules)
	base := state.baseline
	if scope != nil && len(scope) == 0 {
		return baselineResult(base, watched), nil
	}
	if err = admit(ctx, slots); err != nil {
		return result, err
	}
	defer func() { <-slots }()
	running, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	if scope == nil {
		base, err = readBaseline(running, repo, location, rules)
		if err != nil {
			return result, err
		}
	} else {
		base = scopedBaseline(base, scope)
	}
	signals, err := diskStatus(running, location, base, scope, maxFiles)
	if err != nil {
		return result, err
	}
	if err = updateChanges(running, repo, location, base, signals, scope, maxFiles); err != nil {
		return result, err
	}
	state.baseline = base
	return baselineResult(base, watched), nil
}

// baselineResult copies the public slice so one response cannot mutate another.
func baselineResult(base *gitBaseline, watched bool) runnerwire.GitChanges {
	files := make([]runnerwire.GitChange, 0, len(base.files))
	for _, name := range slices.Sorted(maps.Keys(base.files)) {
		files = append(files, base.files[name])
	}
	return runnerwire.GitChanges{
		Repository: true,
		Head:       base.head,
		Files:      files,
		Truncated:  base.truncated,
		Watched:    watched,
	}
}

// retryChanges preserves descriptor exhaustion across repository-library errors.
func retryChanges(
	ctx context.Context,
	state *treeState,
	root string,
	slots chan struct{},
	limit int,
) (runnerwire.GitChanges, error) {
	return cmdsdk.Retry(
		ctx,
		func() (runnerwire.GitChanges, error) { return computeChanges(ctx, state, root, slots, limit) },
	)
}

func (s *treeState) reset(ctx context.Context) {
	s.stop()
	s.ctx, s.cancel = context.WithCancel(ctx)
	s.watch = nil
	s.watchDone = nil
	s.watchPhase = ""
	s.baseline = nil
	s.touched = make(map[string]bool)
	s.whole = false
	s.broken = false
}

func (s *treeState) stopBrokenWatch() {
	s.watchMu.Lock()
	broken := s.broken
	s.watchMu.Unlock()
	if broken {
		s.stop()
		s.watchMu.Lock()
		s.watch = nil
		s.watchPhase = "unavailable"
		s.broken = false
		s.whole = true
		s.watchMu.Unlock()
	}
}

func (s *treeState) changeScope(location *gitLocation, rules []ruleStamp) (scope []string, watched bool) {
	s.watchMu.Lock()
	watched = s.watchPhase == "running" && !s.broken
	whole := s.whole || s.broken
	touched := s.touched
	s.touched = make(map[string]bool)
	s.whole = false
	s.watchMu.Unlock()
	base := s.baseline
	if base != nil && watched && !whole && slices.Equal(base.rules, rules) {
		scope = []string{}
		for path := range touched {
			relative, err := filepath.Rel(location.root, path)
			if err != nil || !pathWithin(path, location.root) {
				continue
			}
			if relative == "." {
				scope = nil
				break
			}
			scope = append(scope, filepath.ToSlash(relative))
		}
		if scope != nil {
			scope = renameScope(base, location, scope)
		}

	}
	return scope, watched
}

func renameScope(base *gitBaseline, location *gitLocation, scope []string) []string {
	for name, signal := range base.indexSignals {
		if signal.from == "" {
			continue
		}
		if inGitScope(name, location.prefix, scope) || inGitScope(signal.from, location.prefix, scope) {
			for _, partner := range []string{name, signal.from} {
				if relative, ok := relativeGitPath(partner, location.prefix); ok {
					scope = append(scope, relative)
				}
			}
		}
	}
	return scope
}

func readBaseline(
	ctx context.Context,
	repo *git.Repository,
	location *gitLocation,
	rules []ruleStamp,
) (*gitBaseline, error) {
	tree, head, hash, err := headEntries(ctx, repo)
	if err != nil {
		return nil, err
	}
	idx, err := repo.Storer.Index()
	if err != nil {
		return nil, err
	}
	signals, tracked, err := indexSignals(ctx, tree, head, idx)
	if err != nil {
		return nil, err
	}
	cfg, err := repo.Config()
	if err != nil {
		return nil, err
	}
	patterns, err := repositoryPatterns(ctx, repo, location)
	if err != nil {
		return nil, err
	}
	base := &gitBaseline{
		head:         hash,
		entries:      head,
		tracked:      tracked,
		indexSignals: signals,
		indexTime:    idx.ModTime.UnixNano(),
		fileMode:     cfg.Raw.Section("core").Option("filemode") != "false",
		patterns:     patterns,
		files:        make(map[string]runnerwire.GitChange),
		rules:        rules,
	}
	return base, nil
}

func scopedBaseline(base *gitBaseline, scope []string) *gitBaseline {
	updated := *base
	updated.files = maps.Clone(base.files)
	base = &updated
	for name := range base.files {
		if slices.ContainsFunc(
			scope,
			func(path string) bool { return name == path || strings.HasPrefix(name, path+"/") },
		) {
			delete(base.files, name)
		}
	}
	return base
}

func updateChanges(
	ctx context.Context,
	repo *git.Repository,
	location *gitLocation,
	base *gitBaseline,
	signals map[string]gitSignal,
	scope []string,
	maxFiles int,
) error {
	names := slices.Sorted(maps.Keys(signals))
	for _, name := range names {
		if err := ctx.Err(); err != nil {
			return err
		}
		signal := signals[name]
		if signal.status() == "" {
			continue
		}
		change, err := judgeChange(ctx, repo, location, base.entries, name, signal)
		if err != nil {
			return err
		}
		if change == nil {
			continue
		}
		base.files[change.Path] = *change
		if scope == nil && len(base.files) >= maxFiles {
			base.truncated = true
			break
		}
	}
	if len(base.files) > maxFiles {
		for _, name := range slices.Sorted(maps.Keys(base.files))[maxFiles:] {
			delete(base.files, name)
		}
		base.truncated = true
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	return nil
}
