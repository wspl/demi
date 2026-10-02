module github.com/wspl/demi

go 1.27

toolchain go1.27.1

require (
	github.com/chromedp/cdproto v0.0.0-20260922220944-a19bff23514f
	github.com/coder/websocket v1.8.15
	github.com/dlclark/regexp2 v1.12.0
	github.com/ebitengine/purego v0.11.1
	github.com/fsnotify/fsnotify v1.10.1
	github.com/go-git/go-git/v5 v5.19.2
	github.com/go-json-experiment/json v0.0.0-20260820222146-c27c302e5fc3
	github.com/google/go-cmp v0.7.0
	github.com/google/nftables v0.3.0
	github.com/google/uuid v1.6.0
	github.com/gowebpki/jcs v1.0.2
	github.com/klauspost/compress v1.20.1
	github.com/nlnwa/whatwg-url v0.6.2
	github.com/santhosh-tekuri/jsonschema/v6 v6.0.3
	github.com/sergi/go-diff v1.4.0
	github.com/tmaxmax/go-sse v0.11.0
	github.com/vishvananda/netlink v1.3.1
	github.com/vishvananda/netns v0.0.5
	github.com/vmihailenco/msgpack/v5 v5.4.1
	go.uber.org/goleak v1.3.0
	golang.org/x/image v0.46.0
	golang.org/x/mod v0.41.0
	golang.org/x/net v0.59.0
	golang.org/x/sync v0.23.0
	golang.org/x/sys v0.48.0
	golang.org/x/text v0.42.0
	golang.org/x/tools v0.50.0
)

require (
	dario.cat/mergo v1.0.0 // indirect
	github.com/Microsoft/go-winio v0.6.2 // indirect
	github.com/ProtonMail/go-crypto v1.1.6 // indirect
	github.com/bits-and-blooms/bitset v1.20.0 // indirect
	github.com/chromedp/sysutil v1.1.0 // indirect
	github.com/cloudflare/circl v1.6.3 // indirect
	github.com/cyphar/filepath-securejoin v0.6.1 // indirect
	github.com/emirpasic/gods v1.18.1 // indirect
	github.com/go-git/gcfg v1.5.1-0.20230307220236-3a3c6141e376 // indirect
	github.com/go-git/go-billy/v5 v5.9.0 // indirect
	github.com/golang/groupcache v0.0.0-20241129210726-2c02b8208cf8 // indirect
	github.com/jbenet/go-context v0.0.0-20150711004518-d14ea06fba99 // indirect
	github.com/kevinburke/ssh_config v1.2.0 // indirect
	github.com/klauspost/cpuid/v2 v2.3.0 // indirect
	github.com/mdlayher/netlink v1.7.3-0.20250113171957-fbb4dce95f42 // indirect
	github.com/mdlayher/socket v0.5.0 // indirect
	github.com/pjbgf/sha1cd v0.6.0 // indirect
	github.com/skeema/knownhosts v1.3.1 // indirect
	github.com/vmihailenco/tagparser/v2 v2.0.0 // indirect
	github.com/xanzy/ssh-agent v0.3.3 // indirect
	golang.org/x/crypto v0.57.0 // indirect
	gopkg.in/warnings.v0 v0.1.2 // indirect
)

replace mvdan.cc/sh/v3 => ./third_party/mvdan-sh
