package edge

import (
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runnerwire"
)

const immutable = "public, max-age=31536000, immutable"

func readRelease(path string) (*runnerwire.RunnerRelease, error) {
	bytes, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	release, err := runnerwire.DecodeRunnerRelease(bytes)
	if err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	return &release, nil
}

func (e *Edge) installer(w http.ResponseWriter, r *http.Request) error {
	if e.state.Site.RunnerReleases == "" {
		w.Header().Set("Content-Type", "text/plain; charset=utf-8")
		w.WriteHeader(503)
		_, err := io.WriteString(w, "Runner releases are not configured on this backend.\n")
		return err
	}
	release, err := readRelease(filepath.Join(e.state.Site.RunnerReleases, "manifest.json"))
	if err != nil {
		return err
	}
	if release == nil {
		return errors.New("the runner release directory has no manifest.json")
	}
	backend := e.state.Site.PublicURL
	if backend == nil {
		if r.Host == "" {
			return apiFailure(400, "invalid_query", "The request names no host")
		}
		scheme := "http"
		if overHTTPS(r) {
			scheme = "https"
		}
		backend, err = url.Parse(scheme + "://" + r.Host + "/")
		if err != nil {
			return apiFailure(400, "invalid_query", "The request's host is no URL")
		}
	}
	backend, err = runners.BackendURL(backend)
	if err != nil {
		return err
	}
	script := runners.ShellScript(backend, *release)
	w.Header().Set("Content-Type", "text/x-shellscript; charset=utf-8")
	if r.URL.Path == "/install.ps1" {
		script = runners.PowerShellScript(backend, *release)
		w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	}
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("Content-Length", strconv.Itoa(len(script)))
	_, err = io.WriteString(w, script)
	return err
}

func (e *Edge) runnerArtifact(w http.ResponseWriter, r *http.Request) error {
	missing := apiFailure(404, "not_found", "No such runner artifact")
	release, target, name := r.PathValue("release"), r.PathValue("target"), r.PathValue("file")
	executable := "demi-runner"
	if strings.Contains(target, "windows") {
		executable += ".exe"
	}
	if e.state.Site.RunnerReleases == "" || !commandwire.IsDigest(release) || !commandwire.IsTarget(target) ||
		name != executable {
		return missing
	}
	directory := filepath.Join(e.state.Site.RunnerReleases, release)
	manifest, err := readRelease(filepath.Join(directory, "manifest.json"))
	if err != nil {
		return err
	}
	if manifest == nil || manifest.Release != release {
		return missing
	}
	if _, ok := manifest.Targets[target]; !ok {
		return missing
	}
	err = immutableFile(w, r, filepath.Join(directory, target, name))
	if errors.Is(err, os.ErrNotExist) {
		return missing
	}
	return err
}

func (e *Edge) nativeArtifact(w http.ResponseWriter, r *http.Request) error {
	artifact, ok, err := e.state.Services.Native.LocalArtifact(r.Context(), r.PathValue("sha256"))
	if err != nil {
		return err
	}
	if !ok {
		return apiFailure(404, "not_found", "No such native artifact")
	}
	w.Header().Set("Content-Type", "application/octet-stream")
	w.Header().Set("Cache-Control", immutable)
	switch artifact := artifact.(type) {
	case *runners.EncodedArtifact:
		w.Header().Set("Content-Encoding", artifacts.ContentCoding)
		w.Header().Set("Content-Length", strconv.Itoa(len(artifact.Bytes)))
		_, err = w.Write(artifact.Bytes)
		return err
	case *runners.PlainArtifact:
		return immutableFile(w, r, artifact.Path)
	}
	return nil
}

func immutableFile(w http.ResponseWriter, r *http.Request, path string) error {
	file, err := os.Open(path)
	if err != nil {
		return err
	}
	// The operation reports IO failures; cleanup has no further recipient.
	defer func() { _ = file.Close() }()
	info, err := file.Stat()
	if err != nil {
		return err
	}
	w.Header().Set("Content-Type", "application/octet-stream")
	w.Header().Set("Cache-Control", immutable)
	w.Header().Set("Content-Length", strconv.FormatInt(info.Size(), 10))
	if r.Method == "HEAD" {
		w.WriteHeader(200)
		return nil
	}
	_, err = io.CopyBuffer(w, file, make([]byte, 256*1024))
	return err
}
