package edge

import (
	"net/http"
	"os"
	"path"
	"path/filepath"
	"strings"
)

func assets(directory string) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		missing := func() {
			noRoute(w, r)
		}
		if directory == "" || r.URL.Path == "/api" || strings.HasPrefix(r.URL.Path, "/api/") {
			missing()
			return
		}
		navigation := path.Ext(r.URL.Path) == "" && strings.Contains(r.Header.Get("Accept"), "text/html")
		if r.Method != "GET" && r.Method != "HEAD" {
			if navigation {
				w.Header().Set("Allow", "GET,HEAD")
				w.WriteHeader(405)
			} else {
				missing()
			}
			return
		}
		for _, component := range strings.Split(strings.ReplaceAll(r.URL.Path, "\\", "/"), "/") {
			if component == ".." {
				missing()
				return
			}
		}

		file := strings.TrimPrefix(path.Clean(r.URL.Path), "/")
		info, err := os.Stat(filepath.Join(directory, filepath.FromSlash(file)))
		if err == nil && info.IsDir() {
			file = path.Join(file, "index.html")
		}
		opened, err := os.Open(filepath.Join(directory, filepath.FromSlash(file)))
		if err != nil && path.Ext(r.URL.Path) == "" && strings.Contains(r.Header.Get("Accept"), "text/html") {
			opened, err = os.Open(filepath.Join(directory, "index.html"))
		}
		if err != nil {
			missing()
			return
		}
		// The operation reports IO failures; cleanup has no further recipient.
		defer func() { _ = opened.Close() }()
		info, err = opened.Stat()
		if err != nil {
			missing()
			return
		}
		http.ServeContent(w, r, info.Name(), info.ModTime(), opened)
	})
}
