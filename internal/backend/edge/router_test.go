package edge

import (
	"net/http"
	"net/http/httptest"
	"os"
	"slices"
	"strings"
	"testing"
)

// The route list in testdata/routes.txt catches an omitted route, changed verb or
// path and accidental extra endpoint independently of route handler implementation.
func TestRoutesMatchReferenceList(t *testing.T) {
	data, err := os.ReadFile("testdata/routes.txt")
	if err != nil {
		t.Fatal(err)
	}
	tree := (&Edge{}).routeTree()
	var got []string
	var collect func(*routeNode, string)
	collect = func(node *routeNode, path string) {
		for method := range node.methods {
			got = append(got, method+" "+path)
		}
		for part, child := range node.literal {
			collect(child, path+"/"+part)
		}
		if node.parameter != nil {
			collect(node.parameter, path+"/{"+node.parameter.name+"}")
		}
	}
	collect(tree, "")
	slices.Sort(got)
	if strings.Join(got, "\n")+"\n" != string(data) {
		t.Fatalf("routes differ:\ngot:\n%s\nwant:\n%s", strings.Join(got, "\n"), data)
	}
}

func TestRoutingKeepsLiteralPriorityAndExactPaths(t *testing.T) {
	tree := &routeNode{}
	answer := func(text string) endpoint {
		return func(w http.ResponseWriter, _ *http.Request) error {
			_, err := w.Write([]byte(text))
			return err
		}
	}
	tree.add("GET /providers/subscription-login/{id}", answer("login"))
	tree.add("GET /providers/{id}/accounts", answer("accounts"))
	handler := tree.handler(http.HandlerFunc(noRoute))
	for _, test := range []struct {
		method, path, want string
		status             int
	}{
		{"GET", "/providers/subscription-login/accounts", "login", 200},
		{"GET", "/providers/p/accounts", "accounts", 200},
		{"POST", "/providers/p/accounts", `{"code":"not_found","message":"No route for POST /providers/p/accounts"}`, 404},
		{"GET", "/providers//accounts", `{"code":"not_found","message":"No route for GET /providers//accounts"}`, 404},
		{"GET", "/providers/p/accounts/", `{"code":"not_found","message":"No route for GET /providers/p/accounts/"}`, 404},
	} {
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, httptest.NewRequest(test.method, test.path, nil))
		if response.Code != test.status || response.Body.String() != test.want {
			t.Fatalf("%s %s: %d %q", test.method, test.path, response.Code, response.Body.String())
		}
	}
}
