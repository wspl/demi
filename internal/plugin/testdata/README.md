# Plugin manifests

`manifests.json` holds every built-in plugin's registration manifest. `TestManifestsRoundTripGolden` decodes it through the generated contract and compares the complete re-encoded, pretty-printed file, preserving field and embedded schema key order. It is a golden file: no test regenerates it.
