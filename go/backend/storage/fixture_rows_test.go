package storage

import (
	"database/sql"
	"encoding/hex"
	"encoding/json/v2"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

// assertRustRows compares every column of every row with the manifest the Rust
// fixture writer emitted, including opaque secrets and agent-owned JSON.
func assertRustRows(t *testing.T, dir string, control *sql.DB) {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(dir, "rows.json"))
	if err != nil {
		t.Fatal(err)
	}
	var manifest map[string]map[string][][]any
	if err = json.Unmarshal(data, &manifest); err != nil {
		t.Fatal(err)
	}
	for path, tables := range manifest {
		db := control
		if path != "control.sqlite" {
			db, err = openDatabase(t.Context(), filepath.Join(dir, path), conversationV1)
			if err != nil {
				t.Fatal(err)
			}
			defer db.Close()
		}
		for table, expected := range tables {
			rows, err := db.QueryContext(t.Context(), "SELECT * FROM "+table+" ORDER BY rowid")
			if err != nil {
				t.Fatal(err)
			}
			columns, err := rows.Columns()
			if err != nil {
				rows.Close()
				t.Fatal(err)
			}
			actual := [][]any{}
			for rows.Next() {
				values := make([]any, len(columns))
				dest := make([]any, len(columns))
				for i := range values {
					dest[i] = &values[i]
				}
				if err = rows.Scan(dest...); err != nil {
					rows.Close()
					t.Fatal(err)
				}
				for i, value := range values {
					switch v := value.(type) {
					case []byte:
						values[i] = map[string]any{"hex": hex.EncodeToString(v)}
					case int64:
						values[i] = float64(v)
					}
				}
				actual = append(actual, values)
			}
			err = rows.Err()
			rows.Close()
			if err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(actual, expected) {
				t.Fatalf("Rust rows differ in %s/%s", path, table)
			}
		}
	}
}
