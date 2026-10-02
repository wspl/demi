package host

import (
	"context"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"strings"

	git "github.com/go-git/go-git/v5"
	"github.com/go-git/go-git/v5/config"
	"github.com/go-git/go-git/v5/plumbing/format/gitignore"
)

// ignorePatterns uses go-git's grammar while retaining filesystem failures.
// ReadPatterns discards errors from ignore files, cannot be canceled and walks
// the entire repository; the Host needs scoped walks and descriptor retry.
func ignorePatterns(ctx context.Context, path string, domain []string) ([]gitignore.Pattern, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	data, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) || errors.Is(err, errNotDirectory) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	var patterns []gitignore.Pattern
	for _, line := range strings.Split(string(data), "\n") {
		line = strings.TrimSuffix(line, "\r")
		if line != "" && !strings.HasPrefix(line, "#") {
			patterns = append(patterns, gitignore.ParsePattern(line, domain))
		}
	}
	return patterns, nil
}

// repositoryPatterns loads excludes in ascending precedence; nested ignore
// files are appended by the same untracked walk that discovers their files.
func repositoryPatterns(ctx context.Context, repo *git.Repository, location *gitLocation) ([]gitignore.Pattern, error) {
	cfg, err := repo.ConfigScoped(config.SystemScope)
	if err != nil {
		return nil, err
	}
	excludes := cfg.Raw.Section("core").Option("excludesfile")
	home, homeErr := os.UserHomeDir()
	if excludes == "" && homeErr == nil {
		configHome := os.Getenv("XDG_CONFIG_HOME")
		if configHome == "" {
			configHome = filepath.Join(home, ".config")
		}
		excludes = filepath.Join(configHome, "git", "ignore")
	}
	if strings.HasPrefix(excludes, "~/") && homeErr == nil {
		excludes = filepath.Join(home, excludes[2:])
	}
	var patterns []gitignore.Pattern
	if excludes != "" {
		patterns, err = ignorePatterns(ctx, excludes, nil)
		if err != nil {
			return nil, err
		}
	}
	local, err := ignorePatterns(ctx, filepath.Join(location.common, "info", "exclude"), nil)
	if err != nil {
		return nil, err
	}
	patterns = append(patterns, local...)
	// A nested requested root still inherits every ignore file above it.
	path := location.workdir
	var domain []string
	if location.prefix != "" {
		for _, component := range strings.Split(location.prefix, "/") {
			local, err = ignorePatterns(ctx, filepath.Join(path, ".gitignore"), domain)
			if err != nil {
				return nil, err
			}
			patterns = append(patterns, local...)
			domain = append(append([]string(nil), domain...), component)
			path = filepath.Join(path, component)
		}
	}
	return patterns, nil
}

// walkUntracked discovers Host files through the repository's ordered ignore
// rules. A whole walk retains those patterns for later changed-path walks.
func walkUntracked(ctx context.Context, location *gitLocation, base *gitBaseline, scope []string, signals map[string]gitSignal, maxFiles int) error {
	roots := []string{location.root}
	patterns := append([]gitignore.Pattern(nil), base.patterns...)
	matcher := gitignore.NewMatcher(patterns)
	if scope != nil {
		roots = nil
		for _, path := range scope {
			roots = append(roots, filepath.Join(location.root, filepath.FromSlash(path)))
		}
	}
	for _, root := range roots {
		if len(signals) > maxFiles {
			base.truncated = true
			break
		}
		err := filepath.WalkDir(root, func(path string, entry fs.DirEntry, failure error) error {
			if err := ctx.Err(); err != nil {
				return err
			}
			if errors.Is(failure, os.ErrNotExist) || errors.Is(failure, errNotDirectory) {
				return nil
			}
			if failure != nil {
				return failure
			}
			if filepath.Base(path) == ".git" {
				if entry.IsDir() {
					return filepath.SkipDir
				}
				return nil
			}
			name, err := filepath.Rel(location.workdir, path)
			if err != nil {
				return err
			}
			name = filepath.ToSlash(name)
			if entry.IsDir() {
				if path != location.root {
					if matcher.Match(strings.Split(name, "/"), true) {
						return filepath.SkipDir
					}
					if _, err := os.Lstat(filepath.Join(path, ".git")); err == nil {
						return filepath.SkipDir
					} else if !errors.Is(err, os.ErrNotExist) && !errors.Is(err, errNotDirectory) {
						return err
					}
				}
				if scope == nil {
					domain := strings.Split(name, "/")
					if name == "." {
						domain = nil
					}
					rules, err := ignorePatterns(ctx, filepath.Join(path, ".gitignore"), domain)
					if err != nil {
						return err
					}
					patterns = append(patterns, rules...)
					matcher = gitignore.NewMatcher(patterns)
				}
				return nil
			}
			if base.tracked[name] != nil || signals[name].conflict != "" {
				return nil
			}
			if !entry.Type().IsRegular() && entry.Type()&os.ModeSymlink == 0 {
				return nil
			}
			if matcher.Match(strings.Split(name, "/"), false) {
				return nil
			}
			signal := signals[name]
			signal.untracked = true
			signals[name] = signal
			if len(signals) > maxFiles {
				base.truncated = true
				return filepath.SkipAll
			}
			return nil
		})
		if err != nil {
			return err
		}
	}
	if scope == nil {
		base.patterns = patterns
	}
	return nil
}
