package storage

import (
	"context"
	"time"

	"github.com/google/uuid"
)

const (
	userSelect = "SELECT id, email, nickname, role, created_at FROM users"
	userInsert = "INSERT INTO users (id, email, password_hash, role, created_at) VALUES (?, ?, ?, ?, ?)"
)

// emailResendCooldown is how long a challenge stands before another may
// replace it.
const emailResendCooldown = time.Minute

// emailChallengeAttempts is how many wrong codes a challenge survives.
const emailChallengeAttempts = 5

func scanUser(row scanner) (User, error) {
	var user User
	err := row.Scan(&user.ID, &user.Email, &user.Nickname, &user.Role, &user.CreatedAt)
	return user, err
}

func newUserRecord(email string, role Role) User {
	return User{
		ID:        uuid.NewString(),
		Email:     email,
		Role:      role,
		CreatedAt: isoTime(time.Now()),
	}
}

func insertUser(ctx context.Context, q Querier, user User, passwordHash string) error {
	_, err := q.ExecContext(ctx, userInsert, user.ID, user.Email, passwordHash, user.Role, user.CreatedAt)
	return err
}

// CreateMaster implements ControlService.
func (c *LocalControlService) CreateMaster(ctx context.Context, email, passwordHash string) (*User, error) {
	record := newUserRecord(email, RoleMaster)
	var created *User
	err := c.db.Transaction(ctx, func(q Querier) error {
		users, err := queryOne(ctx, q, scanInt, "SELECT COUNT(*) FROM users")
		if err != nil {
			return err
		}
		if *users > 0 {
			return nil
		}
		if err := insertUser(ctx, q, record, passwordHash); err != nil {
			return err
		}
		created = &record
		return nil
	})
	return created, err
}

// CreateUser implements ControlService.
func (c *LocalControlService) CreateUser(ctx context.Context, user NewUser) (*User, error) {
	record := newUserRecord(user.Email, user.Role)
	var created *User
	err := c.db.Transaction(ctx, func(q Querier) error {
		existing, err := queryOne(ctx, q, scanUser, userSelect+" WHERE email = ?", user.Email)
		if err != nil || existing != nil {
			return err
		}
		if err := insertUser(ctx, q, record, user.PasswordHash); err != nil {
			return err
		}
		created = &record
		return nil
	})
	return created, err
}

// GetUser implements ControlService.
func (c *LocalControlService) GetUser(ctx context.Context, id string) (*User, error) {
	return read(ctx, c.db, scanUser, userSelect+" WHERE id = ?", id)
}

// FindUserByEmail implements ControlService.
func (c *LocalControlService) FindUserByEmail(ctx context.Context, email string) (*UserWithPassword, error) {
	scan := func(row scanner) (UserWithPassword, error) {
		var user UserWithPassword
		err := row.Scan(&user.ID, &user.Email, &user.Nickname, &user.Role, &user.CreatedAt, &user.PasswordHash)
		return user, err
	}
	return read(ctx, c.db, scan,
		"SELECT id, email, nickname, role, created_at, password_hash FROM users WHERE email = ?", email)
}

// ListUsers implements ControlService.
func (c *LocalControlService) ListUsers(ctx context.Context) ([]User, error) {
	return readAll(ctx, c.db, scanUser, userSelect+" ORDER BY created_at")
}

// CountUsers implements ControlService.
func (c *LocalControlService) CountUsers(ctx context.Context) (int64, error) {
	return c.count(ctx, "SELECT COUNT(*) FROM users")
}

// GetMaster implements ControlService.
func (c *LocalControlService) GetMaster(ctx context.Context) (*User, error) {
	return read(ctx, c.db, scanUser, userSelect+" WHERE role = 'master'")
}

// SetUserNickname implements ControlService.
func (c *LocalControlService) SetUserNickname(ctx context.Context, id, nickname string) error {
	return c.exec(ctx, "UPDATE users SET nickname = ? WHERE id = ?", nickname, id)
}

// SetUserPassword implements ControlService.
func (c *LocalControlService) SetUserPassword(ctx context.Context, id, passwordHash string) error {
	return c.exec(ctx, "UPDATE users SET password_hash = ? WHERE id = ?", passwordHash, id)
}

// IssueEmailChallenge implements ControlService.
func (c *LocalControlService) IssueEmailChallenge(ctx context.Context, challenge EmailChallenge) (bool, error) {
	issued := false
	err := c.db.Transaction(ctx, func(q Querier) error {
		previous, err := queryOne(ctx, q, scanInt, "SELECT sent_at FROM email_challenges WHERE user_id = ?", challenge.UserID)
		if err != nil {
			return err
		}
		if previous != nil && challenge.SentAt.UnixMilli()-*previous < emailResendCooldown.Milliseconds() {
			return nil
		}
		_, err = q.ExecContext(ctx, `INSERT INTO email_challenges (user_id, id, email, password_hash, code_hash, expires_at, sent_at)
        VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(user_id) DO UPDATE SET
        id = excluded.id, email = excluded.email, password_hash = excluded.password_hash,
        code_hash = excluded.code_hash, expires_at = excluded.expires_at, sent_at = excluded.sent_at, attempts = 0`,
			challenge.UserID, challenge.ID, challenge.Email, challenge.PasswordHash, challenge.CodeHash,
			challenge.ExpiresAt.UnixMilli(), challenge.SentAt.UnixMilli())
		issued = err == nil
		return err
	})
	return issued, err
}

// DeleteEmailChallenge implements ControlService.
func (c *LocalControlService) DeleteEmailChallenge(ctx context.Context, userID, id string) error {
	return c.exec(ctx, "DELETE FROM email_challenges WHERE user_id = ? AND id = ?", userID, id)
}

// ConfirmEmailChallenge implements ControlService.
func (c *LocalControlService) ConfirmEmailChallenge(ctx context.Context, userID, id, codeHash string, now time.Time) (EmailChallengeOutcome, error) {
	type pending struct {
		email        string
		passwordHash string
		codeHash     string
		expiresAt    int64
		attempts     int64
	}
	scanPending := func(row scanner) (pending, error) {
		var p pending
		err := row.Scan(&p.email, &p.passwordHash, &p.codeHash, &p.expiresAt, &p.attempts)
		return p, err
	}
	outcome := EmailInvalidCode
	err := c.db.Transaction(ctx, func(q Querier) error {
		challenge, err := queryOne(ctx, q, scanPending,
			"SELECT email, password_hash, code_hash, expires_at, attempts FROM email_challenges WHERE user_id = ? AND id = ?", userID, id)
		if err != nil {
			return err
		}
		passwordHash, err := queryOne(ctx, q, scanString, "SELECT password_hash FROM users WHERE id = ?", userID)
		if err != nil || challenge == nil || passwordHash == nil {
			return err
		}
		// A password change since the challenge was issued voids it.
		if challenge.expiresAt <= now.UnixMilli() || challenge.attempts >= emailChallengeAttempts || challenge.passwordHash != *passwordHash {
			return nil
		}
		if challenge.codeHash != codeHash {
			_, err := q.ExecContext(ctx, "UPDATE email_challenges SET attempts = attempts + 1 WHERE user_id = ?", userID)
			return err
		}
		holder, err := queryOne(ctx, q, scanString, "SELECT id FROM users WHERE email = ? AND id != ?", challenge.email, userID)
		if err != nil {
			return err
		}
		if holder != nil {
			outcome = EmailTaken
			return nil
		}
		if _, err := q.ExecContext(ctx, "UPDATE users SET email = ? WHERE id = ?", challenge.email, userID); err != nil {
			return err
		}
		if _, err := q.ExecContext(ctx, "DELETE FROM email_challenges WHERE user_id = ?", userID); err != nil {
			return err
		}
		outcome = EmailChanged
		return nil
	})
	return outcome, err
}

// CreateWebSession implements ControlService.
func (c *LocalControlService) CreateWebSession(ctx context.Context, session WebSession) error {
	return c.exec(ctx, "INSERT INTO web_sessions (token_hash, user_id, expires_at) VALUES (?, ?, ?)",
		session.TokenHash, session.UserID, isoTime(session.ExpiresAt))
}

// GetWebSession implements ControlService.
func (c *LocalControlService) GetWebSession(ctx context.Context, tokenHash string) (*WebSessionRecord, error) {
	scan := func(row scanner) (WebSessionRecord, error) {
		var session WebSessionRecord
		err := row.Scan(&session.UserID, &session.ExpiresAt)
		return session, err
	}
	return read(ctx, c.db, scan, "SELECT user_id, expires_at FROM web_sessions WHERE token_hash = ?", tokenHash)
}

// ExtendWebSession implements ControlService.
func (c *LocalControlService) ExtendWebSession(ctx context.Context, tokenHash string, expiresAt time.Time) error {
	return c.exec(ctx, "UPDATE web_sessions SET expires_at = ? WHERE token_hash = ?", isoTime(expiresAt), tokenHash)
}

// DeleteWebSession implements ControlService.
func (c *LocalControlService) DeleteWebSession(ctx context.Context, tokenHash string) error {
	return c.exec(ctx, "DELETE FROM web_sessions WHERE token_hash = ?", tokenHash)
}

// DeleteExpiredWebSessions implements ControlService.
func (c *LocalControlService) DeleteExpiredWebSessions(ctx context.Context, before time.Time) error {
	return c.exec(ctx, "DELETE FROM web_sessions WHERE expires_at <= ?", isoTime(before))
}
