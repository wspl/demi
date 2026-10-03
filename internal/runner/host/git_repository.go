package host

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	git "github.com/go-git/go-git/v5"
	"github.com/go-git/go-git/v5/plumbing"
	"github.com/go-git/go-git/v5/plumbing/cache"
	"github.com/go-git/go-git/v5/plumbing/filemode"
	"github.com/go-git/go-git/v5/plumbing/object"
	"github.com/go-git/go-git/v5/storage/filesystem"
	"github.com/go-git/go-git/v5/storage/filesystem/dotgit"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
)

// gitError keeps protocol words apart from wrapped library and IO causes.
type gitError struct {
	code, message string
	cause         error
}

func (e *gitError) Error() string { return e.message }
func (e *gitError) Unwrap() error { return e.cause }

func gitProblem(err error) *gitError {
	var failure *gitError
	if errors.As(err, &failure) {
		return failure
	}
	if errors.Is(err, context.Canceled) {
		return &gitError{"cancelled", "the working-tree request was cancelled", err}
	}
	if errors.Is(err, context.DeadlineExceeded) {
		return &gitError{"timeout", "the working-tree request timed out", err}
	}
	if code := ErrorCode(err); code != nil {
		return &gitError{*code, err.Error(), err}
	}
	var path *os.PathError
	if errors.As(err, &path) {
		return &gitError{"EIO", err.Error(), err}
	}
	return &gitError{"internal", err.Error(), err}
}

type gitLocation struct{ root, workDir, gitDir, common, prefix string }

// openRepository discovers the Host repository without shelling out or hiding IO errors.
func openRepository(root string) (*git.Repository, *gitLocation, error) {
	root, err := repositoryRoot(root)
	if err != nil {
		return nil, nil, err
	}
	repo, err := git.PlainOpenWithOptions(root, &git.PlainOpenOptions{DetectDotGit: true})
	if errors.Is(err, git.ErrRepositoryNotExists) {
		return nil, nil, nil
	}
	if err != nil {
		return nil, nil, err
	}
	work, err := repo.Worktree()
	if errors.Is(err, git.ErrIsBareRepository) {
		return nil, nil, closeRepository(repo)
	}
	if err != nil {
		return nil, nil, errors.Join(err, closeRepository(repo))
	}
	workDir, err := filepath.EvalSymlinks(work.Filesystem.Root())
	if err != nil {
		return nil, nil, errors.Join(err, closeRepository(repo))
	}
	workDir, err = filepath.Abs(workDir)
	if err != nil {
		return nil, nil, errors.Join(err, closeRepository(repo))
	}
	gitDir, common, err := repositoryDirectories(workDir)
	if err != nil {
		return nil, nil, errors.Join(err, closeRepository(repo))
	}
	if common != gitDir {
		repo, err = withCommonDirectory(repo, work, common)
		if err != nil {
			return nil, nil, err
		}
	}
	prefix, err := filepath.Rel(workDir, root)
	if err != nil {
		return nil, nil, errors.Join(err, closeRepository(repo))
	}
	if prefix == "." {
		prefix = ""
	}
	return repo, &gitLocation{
		root:    root,
		workDir: workDir,
		gitDir:  gitDir,
		common:  common,
		prefix:  filepath.ToSlash(prefix),
	}, nil
}

// withCommonDirectory uses go-git's common-directory router while owning the
// discovery file. v5.19.2's PlainOpen EnableDotGitCommonDir leaks its commondir
// handle, so the Host reads that file above and opens both storages explicitly.
func withCommonDirectory(local *git.Repository, work *git.Worktree, common string) (*git.Repository, error) {
	shared, err := git.PlainOpen(common)
	if err != nil {
		return nil, errors.Join(err, closeRepository(local))
	}
	// PlainOpen constructs filesystem.Storage; the Storer interface has no
	// Filesystem method. These assertions check that documented concrete path.
	localStorage, localOK := local.Storer.(*filesystem.Storage)
	sharedStorage, sharedOK := shared.Storer.(*filesystem.Storage)
	if !localOK || !sharedOK {
		return nil, errors.Join(
			errors.New("unexpected Git filesystem storage"),
			closeRepository(local),
			closeRepository(shared),
		)
	}
	storage := filesystem.NewStorage(
		dotgit.NewRepositoryFilesystem(localStorage.Filesystem(), sharedStorage.Filesystem()),
		cache.NewObjectLRUDefault(),
	)
	repo, err := git.Open(storage, work.Filesystem)
	err = errors.Join(err, closeRepository(local), closeRepository(shared))
	if err != nil {
		return nil, errors.Join(err, storage.Close())
	}
	return repo, nil
}

// closeRepository releases go-git's filesystem object storage. v5's Storer
// interface omits Close although its filesystem implementation supplies it.
func closeRepository(repo *git.Repository) error {
	if closer, ok := repo.Storer.(io.Closer); ok {
		return closer.Close()
	}
	return nil
}

// headEntries reads the committed Git tree through go-git's object walker.
func headEntries(
	ctx context.Context,
	repo *git.Repository,
) (*object.Tree, map[string]object.TreeEntry, *string, error) {
	entries := make(map[string]object.TreeEntry)
	ref, err := repo.Head()
	if errors.Is(err, plumbing.ErrReferenceNotFound) {
		return nil, entries, nil, nil
	}
	if err != nil {
		return nil, nil, nil, err
	}
	hash := ref.Hash().String()
	commit, err := repo.CommitObject(ref.Hash())
	if err != nil {
		return nil, nil, nil, err
	}
	tree, err := commit.Tree()
	if err != nil {
		return nil, nil, nil, err
	}
	walk := object.NewTreeWalker(tree, true, nil)
	defer walk.Close()
	for {
		if err = ctx.Err(); err != nil {
			return nil, nil, nil, err
		}
		name, entry, err := walk.Next()
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return nil, nil, nil, err
		}
		if entry.Mode != filemode.Dir {
			entries[name] = entry
		}
	}
	return tree, entries, &hash, nil
}

type gitContent struct {
	bytes          []byte
	present, large bool
}

// diskContent reads the comparison side on disk, treating directories as absent.
func diskContent(ctx context.Context, path string) (gitContent, error) {
	info, err := os.Lstat(path)
	if errors.Is(err, os.ErrNotExist) || errors.Is(err, errNotDirectory) {
		return gitContent{}, nil
	}
	if err != nil {
		return gitContent{}, err
	}
	if info.Mode()&os.ModeSymlink != 0 {
		target, err := os.Readlink(path)
		return gitContent{bytes: []byte(filepath.ToSlash(target)), present: true}, err
	}
	if !info.Mode().IsRegular() {
		return gitContent{}, nil
	}
	if info.Size() > MaxBlobBytes {
		return gitContent{present: true, large: true}, nil
	}
	if err = ctx.Err(); err != nil {
		return gitContent{}, err
	}
	data, err := os.ReadFile(path)
	return gitContent{bytes: data, present: true}, err
}

// blobContent bounds HEAD content before go-git decodes it for display or a pipe.
func blobContent(ctx context.Context, repo *git.Repository, entry object.TreeEntry) (gitContent, error) {
	if entry.Mode != filemode.Regular && entry.Mode != filemode.Executable && entry.Mode != filemode.Symlink &&
		entry.Mode != filemode.Deprecated {
		return gitContent{}, nil
	}
	blob, err := repo.BlobObject(entry.Hash)
	if err != nil {
		return gitContent{}, err
	}
	if blob.Size > MaxBlobBytes {
		return gitContent{present: true, large: true}, nil
	}
	reader, err := blob.Reader()
	if err != nil {
		return gitContent{}, err
	}
	data, err := io.ReadAll(&contextReader{ctx: ctx, reader: reader})
	return gitContent{bytes: data, present: true}, errors.Join(err, reader.Close())
}

// judgeChange preserves the runner's HEAD-to-disk kind and shared line counts.
func judgeChange(
	ctx context.Context,
	repo *git.Repository,
	location *gitLocation,
	head map[string]object.TreeEntry,
	name string,
	signal gitSignal,
) (*runnerwire.GitChange, error) {
	path, ok := relativeGitPath(name, location.prefix)
	if !ok || path == "" {
		return nil, nil
	}
	disk, err := diskContent(ctx, filepath.Join(location.workDir, filepath.FromSlash(name)))
	if err != nil {
		return nil, err
	}
	before, err := blobContent(ctx, repo, head[name])
	if err != nil {
		return nil, err
	}
	change := &runnerwire.GitChange{Path: path, Status: signal.status(), Kind: runnerwire.ChangeKindModified}
	switch {
	case !before.present && !disk.present:
		return nil, nil
	case !before.present:
		change.Kind = runnerwire.ChangeKindAdded
		if from, ok := relativeGitPath(signal.from, location.prefix); signal.from != "" && ok {
			before, err = blobContent(ctx, repo, head[signal.from])
			if err != nil {
				return nil, err
			}
			if before.present {
				change.Kind = runnerwire.ChangeKindRenamed
				change.From = &from
			}
		}
	case !disk.present:
		change.Kind = runnerwire.ChangeKindDeleted
	}
	if !before.large && !disk.large {
		change.Added, change.Removed = process.LineCounts(before.bytes, disk.bytes)
	}
	return change, nil
}

func relativeGitPath(path, prefix string) (string, bool) {
	if prefix == "" {
		return path, true
	}
	if path == prefix {
		return "", true
	}
	return strings.CutPrefix(path, prefix+"/")
}

func repositoryDirectories(workDir string) (string, string, error) {
	gitDir := filepath.Join(workDir, ".git")
	info, err := os.Stat(gitDir)
	if err == nil && !info.IsDir() {
		gitDir, err = readGitDirectory(workDir, gitDir)
	}
	if err != nil {
		return "", "", err
	}
	gitDir, err = filepath.EvalSymlinks(gitDir)
	if err != nil {
		return "", "", err
	}
	common := gitDir
	data, err := os.ReadFile(filepath.Join(gitDir, "commondir"))
	if err == nil {
		common = strings.TrimSpace(string(data))
		if !filepath.IsAbs(common) {
			common = filepath.Join(gitDir, common)
		}
		common, err = filepath.EvalSymlinks(common)
	}
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return "", "", err
	}
	return gitDir, common, nil
}

func readGitDirectory(workDir, gitDir string) (string, error) {
	data, err := os.ReadFile(gitDir)
	if err != nil {
		return gitDir, err
	}
	target, ok := strings.CutPrefix(strings.TrimSpace(string(data)), "gitdir: ")
	if !ok {
		return gitDir, fmt.Errorf("invalid git directory file")
	}
	gitDir = target
	if !filepath.IsAbs(gitDir) {
		gitDir = filepath.Join(workDir, gitDir)
	}
	return gitDir, nil
}

func repositoryRoot(path string) (string, error) {
	root, err := filepath.EvalSymlinks(path)
	if err != nil {
		return "", err
	}
	root, err = filepath.Abs(root)
	if err != nil {
		return "", err
	}
	info, err := os.Stat(root)
	if err != nil {
		return "", err
	}
	if !info.IsDir() {
		return "", &os.PathError{Op: "open", Path: root, Err: errNotDirectory}
	}
	return root, nil
}
