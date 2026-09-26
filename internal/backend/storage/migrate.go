package storage

import (
	"context"
	"fmt"
	"slices"
	"strings"
	"time"
)

// Migration is one numbered schema change.
type Migration struct {
	ID   int
	Name string
	// SQL holds statements separated by semicolons.
	SQL string
}

// Migrate applies the migrations db has not recorded in schema_migrations, in
// id order, each with its record in one transaction. Applying the same set
// again changes nothing.
func Migrate(ctx context.Context, db Database, migrations []Migration) error {
	applied := map[int]bool{}
	err := db.Use(ctx, func(q Querier) error {
		if _, err := q.ExecContext(ctx, "CREATE TABLE IF NOT EXISTS schema_migrations (id INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL)"); err != nil {
			return err
		}
		rows, err := q.QueryContext(ctx, "SELECT id FROM schema_migrations")
		if err != nil {
			return err
		}
		defer rows.Close()
		for rows.Next() {
			var id int
			if err := rows.Scan(&id); err != nil {
				return err
			}
			applied[id] = true
		}
		return rows.Err()
	})
	if err != nil {
		return fmt.Errorf("read applied migrations: %w", err)
	}
	pending := slices.Clone(migrations)
	slices.SortFunc(pending, func(a, b Migration) int { return a.ID - b.ID })
	for _, migration := range pending {
		if applied[migration.ID] {
			continue
		}
		err := db.Transaction(ctx, func(q Querier) error {
			for _, statement := range splitStatements(migration.SQL) {
				if _, err := q.ExecContext(ctx, statement); err != nil {
					return err
				}
			}
			_, err := q.ExecContext(ctx,
				"INSERT INTO schema_migrations (id, name, applied_at) VALUES (?, ?, ?)",
				migration.ID, migration.Name, isoTime(time.Now()))
			return err
		})
		if err != nil {
			return fmt.Errorf("migration %d %s: %w", migration.ID, migration.Name, err)
		}
	}
	return nil
}

// splitStatements splits at every semicolon, as the TypeScript backend does;
// no migration has a semicolon inside a statement.
func splitStatements(sql string) []string {
	var statements []string
	for _, statement := range strings.Split(sql, ";") {
		statement = strings.TrimSpace(statement)
		if statement != "" {
			statements = append(statements, statement)
		}
	}
	return statements
}

// isoTime formats t as JavaScript's Date.toISOString does. Stored timestamps
// compare as text, so every writer must use this one form.
func isoTime(t time.Time) string {
	return t.UTC().Format("2006-01-02T15:04:05.000Z")
}
