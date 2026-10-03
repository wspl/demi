package skills

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"slices"
	"strings"
	"sync"

	"github.com/go-git/go-billy/v5/osfs"
	"github.com/go-git/go-git/v5/plumbing"
	"github.com/go-git/go-git/v5/plumbing/cache"
	"github.com/go-git/go-git/v5/plumbing/filemode"
	"github.com/go-git/go-git/v5/plumbing/format/packfile"
	"github.com/go-git/go-git/v5/plumbing/object"
	"github.com/go-git/go-git/v5/plumbing/protocol/packp"
	"github.com/go-git/go-git/v5/plumbing/protocol/packp/capability"
	"github.com/go-git/go-git/v5/plumbing/storer"
	"github.com/go-git/go-git/v5/plumbing/transport"
	githttp "github.com/go-git/go-git/v5/plumbing/transport/http"
	"github.com/go-git/go-git/v5/storage/filesystem"
)

const (
	fetchMaxBytes = 64 * 1024 * 1024
	filesMaxBytes = 16 * 1024 * 1024
)

var errRepositoryTooLarge = errors.New("the repository is larger than 64 MiB")

type fetched struct {
	commit  string
	skills  []fetchedSkill
	skipped []Skipped
}
type fetchedSkill struct {
	parsed    parsedSkill
	directory string
	files     []fetchedFile
}
type fetchedFile struct {
	path       string
	executable bool
	bytes      []byte
}

// fetch reads just the public default branch's newest commit through go-git's
// per-fetch HTTP session. No process-global Git configuration or client is used.
func fetch(ctx context.Context, url, repository string) (result fetched, err error) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	endpoint, err := transport.NewEndpoint(url)
	if err != nil {
		return result, err
	}
	endpoint.User = ""
	endpoint.Password = ""
	network := &http.Transport{Proxy: http.ProxyFromEnvironment, ForceAttemptHTTP2: true}
	defer network.CloseIdleConnections()
	public := &publicGitTransport{transport: network}
	defer func() { err = errors.Join(err, public.closeBodies()) }()
	client := githttp.NewClient(&http.Client{Transport: public})
	session, err := client.NewUploadPackSession(endpoint, nil)
	if err != nil {
		return result, err
	}
	defer func() { err = errors.Join(err, session.Close()) }()
	head, err := advertisedHead(ctx, session)
	if err != nil {
		return result, err
	}
	request, err := shallowRequest(head)
	if err != nil {
		return result, err
	}
	response, err := session.UploadPack(ctx, request)
	if err != nil {
		return result, err
	}
	defer func() { err = errors.Join(err, response.Close()) }()
	directory, err := os.MkdirTemp("", "demi-skills-")
	if err != nil {
		return result, fmt.Errorf("no temporary directory: %w", err)
	}
	defer func() { err = errors.Join(err, os.RemoveAll(directory)) }()
	store := filesystem.NewStorage(osfs.New(directory), cache.NewObjectLRUDefault())
	defer func() { err = errors.Join(err, store.Close()) }()
	if err := decodeRepositoryPack(ctx, store, response); err != nil {
		return result, err
	}
	return fetchedCommit(ctx, store, head, repository)
}

// publicGitTransport strips even credentials embedded in redirects: a skill
// fetch never sends repository credentials or inherits a cookie jar.
type publicGitTransport struct {
	transport *http.Transport
	mu        sync.Mutex
	bodies    []io.ReadCloser
}

// RoundTrip sends a public Git request and retains its response body for cleanup.
func (t *publicGitTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	request = request.Clone(request.Context())
	request.URL.User = nil
	request.Header.Del("Authorization")
	request.Header.Del("Cookie")
	response, err := t.transport.RoundTrip(request)
	if err != nil {
		return nil, err
	}
	t.mu.Lock()
	t.bodies = append(t.bodies, response.Body)
	t.mu.Unlock()
	return response, nil
}

// closeBodies covers go-git v5's upload-pack decode error paths, which can
// return without closing the HTTP body. net/http response bodies permit repeated
// Close, so this also safely covers bodies closed by successful Git decoding.
func (t *publicGitTransport) closeBodies() error {
	t.mu.Lock()
	bodies := t.bodies
	t.bodies = nil
	t.mu.Unlock()
	var err error
	for _, body := range bodies {
		err = errors.Join(err, body.Close())
	}
	return err
}

// packReader applies Rust's compressed-pack budget before the decoder can read
// beyond it, and checks cancellation during CPU-bound pack decoding as well.
type packReader struct {
	ctx       context.Context
	reader    io.Reader
	remaining int64
}

// Read applies the compressed-pack budget and checks cancellation.
func (r *packReader) Read(p []byte) (int, error) {
	if err := r.ctx.Err(); err != nil {
		return 0, err
	}
	if r.remaining < 0 {
		return 0, errRepositoryTooLarge
	}
	if int64(len(p)) > r.remaining+1 {
		p = p[:r.remaining+1]
	}
	n, err := r.reader.Read(p)
	r.remaining -= int64(n)
	if r.remaining < 0 {
		return n, errRepositoryTooLarge
	}
	return n, err
}

type repositoryFile struct {
	path       string
	hash       plumbing.Hash
	executable bool
}

// repositoryFiles records regular files breadth-first, without following links
// or submodules, preserving the order of Rust's tree traversal.
func repositoryFiles(ctx context.Context, root *object.Tree) ([]repositoryFile, error) {
	type directory struct {
		prefix string
		tree   *object.Tree
	}
	queue := []directory{{tree: root}}
	files := []repositoryFile{}
	for len(queue) > 0 {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		next := queue[0]
		queue = queue[1:]
		for _, entry := range next.tree.Entries {
			path := next.prefix + strings.ToValidUTF8(entry.Name, "\ufffd")
			switch entry.Mode {
			case filemode.Dir:
				tree, err := next.tree.Tree(entry.Name)
				if err != nil {
					return nil, err
				}
				queue = append(queue, directory{prefix: path + "/", tree: tree})
			case filemode.Regular, filemode.Deprecated, filemode.Executable:
				files = append(
					files,
					repositoryFile{path: path, hash: entry.Hash, executable: entry.Mode == filemode.Executable},
				)
			case filemode.Symlink, filemode.Submodule, filemode.Empty:
			}
		}
	}
	return files, nil
}

// skillsOf assigns each regular file to its nearest SKILL.md directory, even
// when a nested manifest is invalid and is reported as skipped.
func skillsOf(
	ctx context.Context,
	store storer.EncodedObjectStorer,
	files []repositoryFile,
	rootName string,
) (fetched, error) {
	directories := map[string]repositoryFile{}
	for _, file := range files {
		if file.path == "SKILL.md" {
			directories[""] = file
		} else if directory, ok := strings.CutSuffix(file.path, "/SKILL.md"); ok {
			directories[directory] = file
		}
	}
	ordered := make([]string, 0, len(directories))
	for directory := range directories {
		ordered = append(ordered, directory)
	}
	slices.Sort(ordered)
	result := fetched{skills: []fetchedSkill{}, skipped: []Skipped{}}
	total := 0
	for _, directory := range ordered {
		if err := ctx.Err(); err != nil {
			return fetched{}, err
		}
		manifest := directories[directory]
		text, err := readRepositoryBlob(ctx, store, manifest.hash, skillMDMaxBytes+1)
		if err != nil {
			return fetched{}, err
		}
		name := directory[strings.LastIndexByte(directory, '/')+1:]
		if name == "" {
			name = rootName
		}
		parsed, err := parseSkill(name, text)
		if err != nil {
			result.skipped = append(result.skipped, Skipped{Path: manifest.path, Reason: err.Error()})
			continue
		}
		if len(result.skills) == sourceMaxSkills {
			return fetched{}, fmt.Errorf("the repository has more than 100 skills")
		}
		skill := fetchedSkill{parsed: parsed, directory: directory, files: []fetchedFile{}}
		if err := collectSkillFiles(ctx, store, files, ordered, &skill, &total); err != nil {
			return fetched{}, err
		}
		result.skills = append(result.skills, skill)
	}
	if len(result.skills) == 0 {
		return fetched{}, fmt.Errorf("the repository holds no skill")
	}
	return result, nil
}

// withinSkill checks the repository path boundary without OS path conversion.
func withinSkill(directory, path string) (string, bool) {
	if directory == "" {
		return path, true
	}
	return strings.CutPrefix(path, directory+"/")
}

func readRepositoryBlob(
	ctx context.Context,
	store storer.EncodedObjectStorer,
	hash plumbing.Hash,
	limit int,
) (content []byte, err error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	blob, err := object.GetBlob(store, hash)
	if err != nil {
		return nil, err
	}
	reader, err := blob.Reader()
	if err != nil {
		return nil, err
	}
	defer func() { err = errors.Join(err, reader.Close()) }()
	return io.ReadAll(io.LimitReader(reader, int64(limit)))
}

func fetchedCommit(
	ctx context.Context,
	store storer.EncodedObjectStorer,
	head plumbing.Hash,
	repository string,
) (fetched, error) {
	commit, err := object.GetCommit(store, head)
	if err != nil {
		return fetched{}, fmt.Errorf("the repository holds no commit")
	}
	tree, err := commit.Tree()
	if err != nil {
		return fetched{}, err
	}
	files, err := repositoryFiles(ctx, tree)
	if err != nil {
		return fetched{}, err
	}
	result, err := skillsOf(ctx, store, files, repository)
	result.commit = commit.Hash.String()
	return result, err
}

func collectSkillFiles(
	ctx context.Context,
	store storer.EncodedObjectStorer,
	files []repositoryFile,
	ordered []string,
	skill *fetchedSkill,
	total *int,
) error {
	for _, file := range files {
		relative, within := withinSkill(skill.directory, file.path)
		if !within {
			continue
		}
		nested := slices.ContainsFunc(ordered, func(other string) bool {
			_, below := withinSkill(skill.directory, other)
			_, contains := withinSkill(other, file.path)
			return other != skill.directory && below && contains
		})
		if nested {
			continue
		}
		content, err := readRepositoryBlob(ctx, store, file.hash, filesMaxBytes-*total+1)
		if err != nil {
			return err
		}
		*total += len(content)
		if *total > filesMaxBytes {
			return fmt.Errorf("the skills' files hold more than 16 MiB")
		}
		skill.files = append(skill.files, fetchedFile{path: relative, executable: file.executable, bytes: content})
	}
	return nil
}

func shallowRequest(head plumbing.Hash) (*packp.UploadPackRequest, error) {
	request := packp.NewUploadPackRequest()
	request.Wants = []plumbing.Hash{head}
	request.Depth = packp.DepthCommits(1)
	if err := request.Capabilities.Set(capability.Shallow); err != nil {
		return nil, err
	}
	return request, nil
}

func advertisedHead(ctx context.Context, session transport.UploadPackSession) (plumbing.Hash, error) {
	advertised, err := session.AdvertisedReferencesContext(ctx)
	if errors.Is(err, transport.ErrEmptyRemoteRepository) {
		return plumbing.ZeroHash, fmt.Errorf("the repository holds no commit")
	}
	if err != nil {
		return plumbing.ZeroHash, err
	}
	if advertised.Head == nil || advertised.Head.IsZero() {
		return plumbing.ZeroHash, fmt.Errorf("the repository holds no commit")
	}
	return *advertised.Head, nil
}

func decodeRepositoryPack(ctx context.Context, store storer.Storer, response io.Reader) error {
	bounded := &packReader{ctx: ctx, reader: response, remaining: fetchMaxBytes}
	if err := packfile.UpdateObjectStorage(store, bounded); err != nil {
		return err
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	return nil
}
