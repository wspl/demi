package edge

import (
	"net/http"
	"net/url"
	"strings"
)

// routeNode gives literal segments precedence over path parameters, as the
// reference router does, without redirecting or cleaning a request's path.
type routeNode struct {
	literal   map[string]*routeNode
	parameter *routeNode
	name      string
	methods   map[string]endpoint
}

func (n *routeNode) add(pattern string, handler endpoint) {
	method, path, _ := strings.Cut(pattern, " ")
	for _, part := range strings.Split(strings.TrimPrefix(path, "/"), "/") {
		if strings.HasPrefix(part, "{") {
			if n.parameter == nil {
				n.parameter = &routeNode{name: strings.Trim(part, "{}")}
			}
			n = n.parameter
		} else {
			if n.literal == nil {
				n.literal = make(map[string]*routeNode)
			}
			if n.literal[part] == nil {
				n.literal[part] = &routeNode{}
			}
			n = n.literal[part]
		}
	}
	if n.methods == nil {
		n.methods = make(map[string]endpoint)
	}
	n.methods[method] = handler
}

func (n *routeNode) match(parts []string, r *http.Request) *routeNode {
	if len(parts) == 0 {
		if n.methods != nil {
			return n
		}
		return nil
	}
	if child := n.literal[parts[0]]; child != nil {
		if found := child.match(parts[1:], r); found != nil {
			return found
		}
	}
	if n.parameter != nil && parts[0] != "" {
		if found := n.parameter.match(parts[1:], r); found != nil {
			value, err := url.PathUnescape(parts[0])
			if err != nil {
				return nil
			}
			r.SetPathValue(n.parameter.name, value)
			return found
		}
	}
	return nil
}

func noRoute(w http.ResponseWriter, r *http.Request) {
	writeError(w, apiFailure(404, "not_found", "No route for "+r.Method+" "+r.URL.EscapedPath()))
}

func (n *routeNode) handler(fallback http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		route := n.match(strings.Split(strings.TrimPrefix(r.URL.EscapedPath(), "/"), "/"), r)
		if route == nil {
			fallback.ServeHTTP(w, r)
			return
		}
		method := r.Method
		if method == "HEAD" && route.methods[method] == nil {
			method = "GET"
		}
		handler := route.methods[method]
		if handler == nil {
			noRoute(w, r)
			return
		}
		serveEndpoint(handler)(w, r)
	})
}
