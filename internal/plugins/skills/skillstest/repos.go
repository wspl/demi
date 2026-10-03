package skillstest

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"maps"
	"net/http"
	"net/http/httptest"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/go-git/go-git/v5/plumbing"
	"github.com/go-git/go-git/v5/plumbing/filemode"
	"github.com/go-git/go-git/v5/plumbing/format/packfile"
	"github.com/go-git/go-git/v5/plumbing/object"
	"github.com/go-git/go-git/v5/plumbing/protocol/packp"
	"github.com/go-git/go-git/v5/plumbing/protocol/packp/capability"
	"github.com/go-git/go-git/v5/storage/memory"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/plugins/skills"
)

// FetchedAt is the millisecond Unix time every fixture fetch ends at.
const FetchedAt int64 = 1790000000000

type fixedClock struct{}

// Now returns the fixture fetch timestamp.
func (fixedClock) Now() core.Timestamp {
	return core.Timestamp(time.UnixMilli(FetchedAt).UTC().Format("2006-01-02T15:04:05.000Z"))
}

// SkillMD wraps front matter in a skill document.
func SkillMD(front string) string { return "---\n" + front + "\n---\n\nFollow these steps.\n" }

// File is one path in a fixture commit. Mode defaults to a regular file;
// setting Executable preserves script permissions. Mode can also model links.
type File struct {
	// Path names the file within the repository.
	Path string
	// Bytes holds the file contents.
	Bytes []byte
	// Executable requests executable file permissions.
	Executable bool
	// Mode overrides the default regular file mode.
	Mode filemode.FileMode
}

type revision struct {
	store   *memory.Storage
	head    plumbing.Hash
	objects []plumbing.Hash
	files   []File
}

// Repos serves shallow smart-HTTP Git repositories, with no Git executable.
// Commits are immutable snapshots, so a test can update while a fetch runs.
type Repos struct {
	mu           sync.Mutex
	repositories map[string]*revision
	server       *httptest.Server
	// BeforeUpload may gate a request on channels for deterministic cancellation
	// tests. Configure before any requests. It must obey the request context.
	BeforeUpload func(context.Context, string) error
}

// New starts a fixture HTTP server and registers its cleanup with the test.
func New(t testing.TB) *Repos {
	t.Helper()
	repos := &Repos{repositories: map[string]*revision{}}
	repos.server = httptest.NewServer(http.HandlerFunc(repos.serveHTTP))
	t.Cleanup(repos.server.Close)
	return repos
}

// Factory fetches these repositories by their public owner/repo names.
func (r *Repos) Factory() (*skills.Factory, error) {
	return skills.NewResolving(
		func(url string) string { return r.server.URL + "/" + strings.TrimPrefix(url, "https://github.com/") },
		fixedClock{},
	)
}

// URL returns the local HTTP endpoint of a fixture repository.
func (r *Repos) URL(name string) string { return r.server.URL + "/" + name }

// Commit replaces the entire tree and retains a parent commit reference.
func (r *Repos) Commit(ctx context.Context, name string, files []File) (string, error) {
	if err := ctx.Err(); err != nil {
		return "", err
	}
	r.mu.Lock()
	previous := r.repositories[name]
	r.mu.Unlock()
	next := newRevision(files)
	root := &fixtureTree{directories: map[string]*fixtureTree{}}
	for _, file := range files {
		if err := ctx.Err(); err != nil {
			return "", err
		}
		components := strings.Split(file.Path, "/")
		tree := root
		for _, component := range components[:len(components)-1] {
			if tree.directories[component] == nil {
				tree.directories[component] = &fixtureTree{directories: map[string]*fixtureTree{}}
			}
			tree = tree.directories[component]
		}
		if err := tree.addFile(next.store, components[len(components)-1], file); err != nil {
			return "", err
		}
	}
	treeHash, err := root.store(ctx, next.store)
	if err != nil {
		return "", err
	}
	signature := object.Signature{Name: "test", Email: "test@example.test", When: time.UnixMilli(FetchedAt)}
	commit := object.Commit{Author: signature, Committer: signature, TreeHash: treeHash, Message: "skills"}
	if previous != nil {
		commit.ParentHashes = []plumbing.Hash{previous.head}
	}
	encoded := &plumbing.MemoryObject{}
	if err := commit.Encode(encoded); err != nil {
		return "", err
	}
	head, err := next.store.SetEncodedObject(encoded)
	if err != nil {
		return "", err
	}
	next.head = head
	next.objects = slices.Collect(maps.Keys(next.store.Objects))
	plumbing.HashesSort(next.objects)
	r.mu.Lock()
	r.repositories[name] = next
	r.mu.Unlock()
	return head.String(), nil
}

// CommitBytes replaces one file while keeping the rest of the current tree.
func (r *Repos) CommitBytes(ctx context.Context, name, path string, content []byte) (string, error) {
	r.mu.Lock()
	current := r.repositories[name]
	r.mu.Unlock()
	if current == nil {
		return "", fmt.Errorf("no fixture repository %q", name)
	}
	files := slices.Clone(current.files)
	index := slices.IndexFunc(files, func(file File) bool { return file.Path == path })
	file := File{Path: path, Bytes: content}
	if index < 0 {
		files = append(files, file)
	} else {
		files[index] = file
	}
	return r.Commit(ctx, name, files)
}

type fixtureTree struct {
	files       []object.TreeEntry
	directories map[string]*fixtureTree
}

func (t *fixtureTree) store(ctx context.Context, storage *memory.Storage) (plumbing.Hash, error) {
	if err := ctx.Err(); err != nil {
		return plumbing.ZeroHash, err
	}
	entries := slices.Clone(t.files)
	for _, name := range slices.Sorted(maps.Keys(t.directories)) {
		hash, err := t.directories[name].store(ctx, storage)
		if err != nil {
			return plumbing.ZeroHash, err
		}
		entries = append(entries, object.TreeEntry{Name: name, Mode: filemode.Dir, Hash: hash})
	}
	// Git tree ordering compares directory names as if suffixed with '/'.
	slices.SortFunc(entries, func(a, b object.TreeEntry) int {
		left, right := a.Name, b.Name
		if a.Mode == filemode.Dir {
			left += "/"
		}
		if b.Mode == filemode.Dir {
			right += "/"
		}
		return strings.Compare(left, right)
	})
	tree := object.Tree{Entries: entries}
	encoded := &plumbing.MemoryObject{}
	if err := tree.Encode(encoded); err != nil {
		return plumbing.ZeroHash, err
	}
	return storage.SetEncodedObject(encoded)
}

func (r *Repos) serveHTTP(w http.ResponseWriter, request *http.Request) {
	if err := r.serve(w, request); err != nil {
		// Once streaming starts, a disconnected client has no error response to read.
		// Tests observe the client's result; the handler owns no detached work.
		return
	}
}

func (r *Repos) serve(w http.ResponseWriter, request *http.Request) error {
	path := strings.TrimPrefix(request.URL.Path, "/")
	name, advertisement := strings.CutSuffix(path, "/info/refs")
	if !advertisement {
		var ok bool
		name, ok = strings.CutSuffix(path, "/git-upload-pack")
		if !ok {
			http.NotFound(w, request)
			return nil
		}
	}
	r.mu.Lock()
	revision := r.repositories[name]
	r.mu.Unlock()
	if revision == nil {
		http.NotFound(w, request)
		return nil
	}
	if request.Header.Get("Authorization") != "" || request.Header.Get("Cookie") != "" {
		http.Error(w, "credential sent", http.StatusForbidden)
		return nil
	}
	if advertisement {
		return advertiseRevision(w, revision)
	}
	upload := packp.NewUploadPackRequest()
	if err := upload.Decode(request.Body); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return err
	}
	if _, err := io.Copy(io.Discard, request.Body); err != nil {
		return err
	}
	if upload.Depth != packp.DepthCommits(1) || len(upload.Wants) != 1 || upload.Wants[0] != revision.head {
		http.Error(w, "fixture requires one shallow HEAD", http.StatusBadRequest)
		return nil
	}
	if r.BeforeUpload != nil {
		if err := r.BeforeUpload(request.Context(), name); err != nil {
			return err
		}
	}
	w.Header().Set("Content-Type", "application/x-git-upload-pack-result")
	if err := (&packp.ShallowUpdate{Shallows: []plumbing.Hash{revision.head}}).Encode(w); err != nil {
		return err
	}
	if err := (&packp.ServerResponse{}).Encode(w, false); err != nil {
		return err
	}
	// The go-git server does not implement shallow traversal. The fixture uses
	// its protocol and pack encoder, sending only this snapshot's objects.
	_, err := packfile.NewEncoder(contextWriter{request.Context(), w}, revision.store, false).
		Encode(revision.objects, 0)
	return err
}

type contextWriter struct {
	ctx    context.Context
	writer io.Writer
}

// Write checks request cancellation before writing the response.
func (w contextWriter) Write(bytes []byte) (int, error) {
	if err := w.ctx.Err(); err != nil {
		return 0, err
	}
	return w.writer.Write(bytes)
}

func (t *fixtureTree) addFile(storage *memory.Storage, name string, file File) error {
	blob := &plumbing.MemoryObject{}
	blob.SetType(plumbing.BlobObject)
	writer, err := blob.Writer()
	if err != nil {
		return err
	}
	if _, err := writer.Write(file.Bytes); err != nil {
		return errors.Join(err, writer.Close())
	}
	if err := writer.Close(); err != nil {
		return err
	}
	hash, err := storage.SetEncodedObject(blob)
	if err != nil {
		return err
	}
	mode := file.Mode
	if mode == filemode.Empty {
		mode = filemode.Regular
	}
	if file.Executable {
		mode = filemode.Executable
	}
	t.files = append(t.files, object.TreeEntry{Name: name, Mode: mode, Hash: hash})
	return nil
}

func advertiseRevision(w http.ResponseWriter, revision *revision) error {
	refs := packp.NewAdvRefs()
	refs.Prefix = [][]byte{[]byte("# service=git-upload-pack\n"), {}}
	refs.Head = &revision.head
	refs.References["refs/heads/main"] = revision.head
	if err := refs.Capabilities.Set(capability.Shallow); err != nil {
		return err
	}
	if err := refs.Capabilities.Set(capability.SymRef, "HEAD:refs/heads/main"); err != nil {
		return err
	}
	w.Header().Set("Content-Type", "application/x-git-upload-pack-advertisement")
	return refs.Encode(w)
}

func newRevision(files []File) *revision {
	next := &revision{store: memory.NewStorage(), files: make([]File, len(files))}
	for i, file := range files {
		next.files[i] = file
		next.files[i].Bytes = bytes.Clone(file.Bytes)
	}
	return next
}
