package main

import (
	"bytes"
	"fmt"
	"testing"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/artifacts/artifactstest"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/contract"
)

func TestPinnedVersionRecordsEachOfficialArchiveWithExecutable(t *testing.T) {
	const version = "153.0.8010.36"
	for _, scenario := range []string{"complete", "unofficial", "missing executable", "wrong version", "invalid argument", "missing archive", "null metadata"} {
		t.Run(scenario, func(t *testing.T) {
			a := appFixture(t)
			client := artifacts.NewClientAllowingHTTP()
			defer client.Close()
			files := make(map[string]artifactstest.Answer)
			for _, platform := range chromePlatforms {
				entry := platform.Executable
				if scenario == "missing executable" && platform.Name == "linux64" {
					entry = "elsewhere"
				}
				files["/"+platform.Name] = artifactstest.OK(artifactstest.Zip(t, map[string][]byte{entry: []byte(platform.Name)}))
			}
			archives := artifactstest.Start(t, files)
			type download struct {
				Platform string `json:"platform"`
				URL      string `json:"url"`
			}
			type downloads struct {
				Chrome []download `json:"chrome"`
			}
			metadata := struct {
				Version   string    `json:"version"`
				Downloads downloads `json:"downloads"`
			}{Version: version}
			for _, platform := range chromePlatforms {
				if scenario == "missing archive" && platform.Name == "linux64" {
					continue
				}
				metadata.Downloads.Chrome = append(metadata.Downloads.Chrome, download{platform.Name, archives.URL("/" + platform.Name)})
			}
			if scenario == "wrong version" {
				metadata.Version = "153.0.8010.37"
			}
			body, err := contract.EncodeJSON(metadata)
			if err != nil {
				t.Fatal(err)
			}
			if scenario == "null metadata" {
				body = []byte(`{"version":"153.0.8010.36","downloads":{"chrome":null}}`)
			}
			server := artifactstest.Start(t, map[string]artifactstest.Answer{"/" + version + ".json": artifactstest.OK(body)})
			official := archives.URL("/")
			if scenario == "unofficial" {
				official = "https://storage.googleapis.com/"
			}
			argument := version
			if scenario == "invalid argument" {
				argument = "153.0.8010"
			}
			data, err := a.prepareBrowser(t.Context(), client, server.URL(""), official, argument)
			if scenario != "complete" {
				if err == nil {
					t.Fatal("invalid release accepted")
				}
				if (scenario == "wrong version" || scenario == "invalid argument") && archives.Requests() != 0 {
					t.Fatal("downloaded before checking version")
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			release, err := browserop.DecodeBrowserRelease(data)
			if err != nil {
				t.Fatal(err)
			}
			if release.Version != version || len(release.Platforms) != 5 || archives.Requests() != 5 {
				t.Fatal(release)
			}
			for i, recorded := range release.Platforms {
				platform := chromePlatforms[i]
				if recorded.Target != platform.Target || recorded.Executable != platform.Executable || recorded.URL != archives.URL("/"+platform.Name) {
					t.Fatal(recorded)
				}
				file := files["/"+platform.Name].Body
				verifier := artifacts.NewVerifier(artifacts.Digest{Size: recorded.Size, SHA256: recorded.SHA256})
				if err := verifier.Update(file); err != nil {
					t.Fatal(err)
				}
				if err := verifier.Finish(); err != nil {
					t.Fatal(err)
				}
			}
			if !bytes.HasSuffix(data, []byte("\n")) || !bytes.Contains(data, []byte(fmt.Sprintf("  \"version\": %q", version))) {
				t.Fatal("wrong record formatting")
			}
		})
	}
}
