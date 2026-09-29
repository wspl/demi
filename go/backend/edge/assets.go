package edge

import (
	"io"
	"io/fs"
	"net/http"
	"os"
	"path"
	"strings"
)

// assets serves the browser build beside the API (web-api.md § Serving the
// browser build): files as they are, and index.html for an extensionless
// navigation that accepts HTML, so a deep page reloads. Any other miss is a
// JSON 404, and without a directory every path the routes do not answer is
// one.
func (s *Server) assets(directory string) http.Handler {
	if directory == "" {
		return answerNoRoute
	}
	return handler(func(w http.ResponseWriter, r *http.Request) error {
		// A root keeps every name, a symbolic link's included, inside the
		// directory.
		root, err := os.OpenRoot(directory)
		if err != nil {
			return noRoute(r.Method, r.URL.Path)
		}
		defer root.Close()
		files := root.FS()
		if r.Method == http.MethodGet || r.Method == http.MethodHead {
			if served := serveFile(w, r, files, strings.TrimPrefix(path.Clean(r.URL.Path), "/")); served {
				return nil
			}
		}
		navigation := path.Ext(r.URL.Path) == "" && strings.Contains(r.Header.Get("Accept"), "text/html")
		if !navigation || !serveFile(w, r, files, "index.html") {
			return noRoute(r.Method, r.URL.Path)
		}
		return nil
	})
}

// serveFile answers the regular file name of files, or the index.html of
// the directory name, and says whether it did.
func serveFile(w http.ResponseWriter, r *http.Request, files fs.FS, name string) bool {
	if name == "" {
		name = "."
	}
	info, err := fs.Stat(files, name)
	if err == nil && info.IsDir() {
		name = path.Join(name, "index.html")
		info, err = fs.Stat(files, name)
	}
	if err != nil || !info.Mode().IsRegular() {
		return false
	}
	file, err := files.Open(name)
	if err != nil {
		return false
	}
	defer file.Close()
	content, ok := file.(io.ReadSeeker)
	if !ok {
		return false
	}
	http.ServeContent(w, r, name, info.ModTime(), content)
	return true
}
