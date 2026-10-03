package skills

import (
	"crypto/sha256"
	"fmt"
	"strings"
)

// origin identifies the public repository independently of its spelling.
type origin struct{ url string }

func parseOrigin(text string) (origin, error) {
	text = strings.TrimSpace(text)
	if rest, ok := strings.CutPrefix(text, "https://"); ok {
		host, path, _ := strings.Cut(rest, "/")
		if host == "" || strings.Trim(path, "/") == "" {
			return origin{}, fmt.Errorf("\"%s\" names no repository", text)
		}
		path = strings.TrimSuffix(strings.TrimRight(path, "/"), ".git")
		return origin{url: "https://" + asciiLower(host) + "/" + path}, nil
	}
	owner, repo, ok := strings.Cut(text, "/")
	repo = strings.TrimSuffix(repo, ".git")
	if ok && githubName(owner) && githubName(repo) {
		return origin{url: "https://github.com/" + owner + "/" + repo}, nil
	}
	return origin{}, fmt.Errorf("\"%s\" is neither owner/repo nor an https URL", text)
}

func (o origin) id() string         { return fmt.Sprintf("%x", sha256.Sum256([]byte(o.url)))[:12] }
func (o origin) repository() string { return o.url[strings.LastIndexByte(o.url, '/')+1:] }

func githubName(part string) bool {
	if part == "" {
		return false
	}
	for _, c := range []byte(part) {
		if !asciiAlphanumeric(c) && c != '-' && c != '_' && c != '.' {
			return false
		}
	}
	return true
}

// asciiLower folds only the ASCII letters A-Z to lowercase; every other character keeps its case.
func asciiLower(text string) string {
	return strings.Map(func(c rune) rune {
		if c >= 'A' && c <= 'Z' {
			return c + ('a' - 'A')
		}
		return c
	}, text)
}

func asciiAlphanumeric(c byte) bool {
	return c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9'
}
