package database

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"math"

	"github.com/google/uuid"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// Close closes the database; operations after it fail with ErrClosed.
func (c *ControlService) Close(ctx context.Context) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	c.mu.Lock()
	c.closed = true
	for c.active != 0 {
		changed := c.changed
		c.mu.Unlock()
		<-changed
		c.mu.Lock()
	}
	c.mu.Unlock()
	return sqlError(c.db.Close())
}

// HasUsers reports whether the instance has any accounts.
func (c *ControlService) HasUsers(ctx context.Context) (bool, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (bool, error) {
		var found bool
		err := tx.QueryRowContext(ctx, "SELECT EXISTS (SELECT 1 FROM users)").Scan(&found)
		return found, err
	})
}

// CreateMaster returns the instance's first account, created only while there is no account
// at all; ErrAlreadySetUp once setup has run.
func (c *ControlService) CreateMaster(
	ctx context.Context,
	email webapiproto.EmailAddress,
	passwordHash PasswordHash,
) (webapiproto.UserDTO, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (webapiproto.UserDTO, error) {
		var exists bool
		if err := tx.QueryRowContext(ctx, "SELECT EXISTS (SELECT 1 FROM users)").Scan(&exists); err != nil {
			return webapiproto.UserDTO{}, err
		}
		if exists {
			return webapiproto.UserDTO{}, ErrAlreadySetUp
		}
		u := webapiproto.UserDTO{
			ID:        webapiproto.UserID(uuid.NewString()),
			Email:     email,
			Role:      webapiproto.RoleMaster,
			CreatedAt: now,
		}
		at, err := now.Millisecond()
		if err != nil {
			return webapiproto.UserDTO{}, err
		}
		result, err := tx.ExecContext(
			ctx,
			`INSERT INTO users (id,email,nickname,password_hash,role,created_at)
VALUES (?,?,'',?,?,?)
ON CONFLICT (email) DO NOTHING`,
			u.ID,
			email,
			passwordHash.Text(),
			u.Role,
			at,
		)
		if err != nil {
			return webapiproto.UserDTO{}, err
		}
		count, err := result.RowsAffected()
		if err != nil {
			return webapiproto.UserDTO{}, err
		}
		if count == 0 {
			return webapiproto.UserDTO{}, ErrAlreadySetUp
		}
		return u, nil
	})
}

// AccountByEmail returns the login lookup.
func (c *ControlService) AccountByEmail(ctx context.Context, email webapiproto.EmailAddress) (Account, bool, error) {
	var found bool
	record, err := controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (Account, error) {
		r, ok, err := queryRecord(
			ctx,
			tx,
			"users",
			"SELECT "+userColumns+", password_hash FROM users WHERE email = ?",
			accountRow,
			email,
		)
		found = ok
		return r, err
	})
	return record, found && err == nil, err
}

// Account returns the account of user, or nil when absent.
func (c *ControlService) Account(ctx context.Context, user webapiproto.UserID) (Account, bool, error) {
	var found bool
	record, err := controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (Account, error) {
		r, ok, err := queryRecord(
			ctx,
			tx,
			"users",
			"SELECT "+userColumns+", password_hash FROM users WHERE id = ?",
			accountRow,
			user,
		)
		found = ok
		return r, err
	})
	return record, found && err == nil, err
}

// Users returns every account, in the order they were created.
func (c *ControlService) Users(ctx context.Context) ([]webapiproto.UserDTO, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) ([]webapiproto.UserDTO, error) {
		return queryRecords(ctx, tx, "users", "SELECT "+userColumns+" FROM users ORDER BY created_at, rowid", userRow)
	})
}

// CreateUser returns a new account of `role`; ErrEmailTaken, writing nothing, when an account has
// the address already.
func (c *ControlService) CreateUser(
	ctx context.Context,
	email webapiproto.EmailAddress,
	passwordHash PasswordHash,
	role webapiproto.Role,
) (webapiproto.UserDTO, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (webapiproto.UserDTO, error) {
		u := webapiproto.UserDTO{ID: webapiproto.UserID(uuid.NewString()), Email: email, Role: role, CreatedAt: now}
		at, err := now.Millisecond()
		if err != nil {
			return webapiproto.UserDTO{}, err
		}
		result, err := tx.ExecContext(
			ctx,
			`INSERT INTO users (id,email,nickname,password_hash,role,created_at)
VALUES (?,?,'',?,?,?)
ON CONFLICT (email) DO NOTHING`,
			u.ID,
			email,
			passwordHash.Text(),
			u.Role,
			at,
		)
		if err != nil {
			return webapiproto.UserDTO{}, err
		}
		count, err := result.RowsAffected()
		if err != nil {
			return webapiproto.UserDTO{}, err
		}
		if count == 0 {
			return webapiproto.UserDTO{}, ErrEmailTaken
		}
		return u, nil
	})
}

// EmailInUse reports whether an account already has email.
func (c *ControlService) EmailInUse(ctx context.Context, email webapiproto.EmailAddress) (bool, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (bool, error) {
		var found bool
		err := tx.QueryRowContext(ctx, "SELECT EXISTS (SELECT 1 FROM users WHERE email = ?)", email).Scan(&found)
		return found, err
	})
}

// SetNickname sets the nickname and answers the account as it now is.
func (c *ControlService) SetNickname(
	ctx context.Context,
	user webapiproto.UserID,
	nickname string,
) (webapiproto.UserDTO, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (webapiproto.UserDTO, error) {
		u, found, err := queryRecord(
			ctx,
			tx,
			"users",
			"UPDATE users SET nickname = ? WHERE id = ? RETURNING "+userColumns,
			userRow,
			nickname,
			user,
		)
		if err != nil {
			return webapiproto.UserDTO{}, err
		}
		if !found {
			return webapiproto.UserDTO{}, ErrUserNotFound
		}
		return u, nil
	})
}

// SetPassword replaces the account password hash.
func (c *ControlService) SetPassword(ctx context.Context, user webapiproto.UserID, passwordHash PasswordHash) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) error {
		return execSQL(ctx, tx, "UPDATE users SET password_hash = ? WHERE id = ?", passwordHash.Text(), user)
	})
}

// Preferences returns the user's saved preferences; a user who saved none has no overrides.
func (c *ControlService) Preferences(ctx context.Context, user webapiproto.UserID) (webapiproto.Preferences, error) {
	return controlCall(
		ctx,
		c,
		func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (webapiproto.Preferences, error) {
			return savedPreferences(ctx, tx, user)
		},
	)
}

// PatchPreferences merges a validated patch in the transaction that reads and
// writes the user's preferences. Absent fields stay; null shortcuts remove overrides.
func (c *ControlService) PatchPreferences(
	ctx context.Context,
	user webapiproto.UserID,
	patch webapiproto.PreferencesPatch,
) (webapiproto.Preferences, error) {
	return controlCall(
		ctx,
		c,
		func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (webapiproto.Preferences, error) {
			p, err := savedPreferences(ctx, tx, user)
			if err != nil {
				return p, err
			}
			mergePreferences(&p, patch)
			text, err := encoded(p)
			if err != nil {
				return p, err
			}
			return p, execSQL(
				ctx,
				tx,
				`INSERT INTO user_preferences (user_id,preferences)
VALUES (?,?)
ON CONFLICT (user_id) DO UPDATE
SET preferences = excluded.preferences`,
				user,
				text,
			)
		},
	)
}

// OpenWebSession stores a new session and answers when it expires. A login is the one
// moment the table grows, so it also drops the sessions that ran out
// unnoticed.
func (c *ControlService) OpenWebSession(
	ctx context.Context,
	token TokenHash,
	user webapiproto.UserID,
	policy SessionPolicy,
) (types.Timestamp, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (types.Timestamp, error) {
		expiry, err := later(now, policy.Lifetime)
		if err != nil {
			return "", err
		}
		at, err := now.Millisecond()
		if err != nil {
			return "", err
		}
		end, err := expiry.Millisecond()
		if err != nil {
			return "", err
		}
		if err := execSQL(ctx, tx, "DELETE FROM web_sessions WHERE expires_at <= ?", at); err != nil {
			return "", err
		}
		return expiry, execSQL(
			ctx,
			tx,
			"INSERT INTO web_sessions (token_hash,user_id,expires_at) VALUES (?,?,?)",
			token.Text(),
			user,
			end,
		)
	})
}

// ResolveWebSession returns the live session a token hash names, renewed when less than the
// policy's margin remains; an expired session is deleted and is nil.
func (c *ControlService) ResolveWebSession(
	ctx context.Context,
	token TokenHash,
	policy SessionPolicy,
) (ResolvedSession, bool, error) {
	var found bool
	r, err := controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (ResolvedSession, error) {
		session, ok, err := queryRecord(
			ctx,
			tx,
			"web_sessions",
			`SELECT s.expires_at,u.id,u.email,u.nickname,u.role,u.created_at
FROM web_sessions s
JOIN users u ON u.id = s.user_id
WHERE s.token_hash = ?`,
			func(r *storedRow) ResolvedSession {
				return ResolvedSession{User: userRow(r), ExpiresAt: r.instant("expires_at")}
			},
			token.Text(),
		)
		if err != nil || !ok {
			return ResolvedSession{}, err
		}
		nowTime, err := now.Time()
		if err != nil {
			return ResolvedSession{}, err
		}
		end, err := session.ExpiresAt.Time()
		if err != nil {
			return ResolvedSession{}, err
		}
		if !end.After(nowTime) {
			return ResolvedSession{},
				execSQL(ctx, tx, "DELETE FROM web_sessions WHERE token_hash = ?", token.Text())
		}
		if end.Sub(nowTime) < policy.RenewBelow {
			expiry, err := later(now, policy.Lifetime)
			if err != nil {
				return ResolvedSession{}, err
			}
			ms, err := expiry.Millisecond()
			if err != nil {
				return ResolvedSession{}, err
			}
			if err := execSQL(
				ctx,
				tx,
				"UPDATE web_sessions SET expires_at = ? WHERE token_hash = ?",
				ms,
				token.Text(),
			); err != nil {
				return ResolvedSession{}, err
			}
			session.ExpiresAt = expiry
			session.Renewed = true
		}
		found = true
		return session, nil
	})
	return r, found && err == nil, err
}

// CloseWebSession deletes the session named by its token hash.
func (c *ControlService) CloseWebSession(ctx context.Context, token TokenHash) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) error {
		return execSQL(ctx, tx, "DELETE FROM web_sessions WHERE token_hash = ?", token.Text())
	})
}

// IssueEmailChallenge stores a challenge in place of the user's previous one and answers
// when it expires; nil while the previous one was sent less than the
// cooldown ago.
func (c *ControlService) IssueEmailChallenge(
	ctx context.Context,
	issue ChallengeIssue,
	policy ChallengePolicy,
) (types.Timestamp, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (types.Timestamp, error) {
		var sent int64
		err := tx.QueryRowContext(ctx, "SELECT sent_at FROM email_challenges WHERE user_id = ?", issue.User).Scan(&sent)
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return "", err
		}
		at, errTime := now.Millisecond()
		if errTime != nil {
			return "", errTime
		}
		if err == nil {
			cooling, err := emailChallengeCooling(sent, now, policy)
			if err != nil {
				return "", err
			}
			if cooling {
				return "", ErrCoolingDown
			}
		}
		expiry, err := later(now, policy.Lifetime)
		if err != nil {
			return "", err
		}
		end, err := expiry.Millisecond()
		if err != nil {
			return "", err
		}
		err = execSQL(
			ctx,
			tx,
			`INSERT INTO email_challenges (user_id,id,email,password_hash,code_hash,expires_at,sent_at,attempts)
VALUES (?,?,?,?,?,?,?,0)
ON CONFLICT (user_id) DO UPDATE
SET
    id=excluded.id,
    email=excluded.email,
    password_hash=excluded.password_hash,
    code_hash=excluded.code_hash,
    expires_at=excluded.expires_at,
    sent_at=excluded.sent_at,
    attempts=0`,
			issue.User,
			issue.ID,
			issue.Email,
			issue.PasswordHash.Text(),
			issue.CodeHash.Text(),
			end,
			at,
		)
		return expiry, err
	})
}

// DeleteEmailChallenge deletes the user's challenge when its ID matches.
func (c *ControlService) DeleteEmailChallenge(ctx context.Context, user webapiproto.UserID, id string) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) error {
		return execSQL(ctx, tx, "DELETE FROM email_challenges WHERE user_id = ? AND id = ?", user, id)
	})
}

// ConfirmEmailChallenge checks the code and, when it holds, changes the address and consumes
// the challenge, in one transaction with the uniqueness check.
func (c *ControlService) ConfirmEmailChallenge(
	ctx context.Context,
	user webapiproto.UserID,
	id string,
	codeHash CodeHash,
	attempts uint32,
) (webapiproto.UserDTO, error) {
	var refusal error
	changed, err := controlCall(
		ctx,
		c,
		func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (webapiproto.UserDTO, error) {
			found, ok, err := queryRecord(
				ctx,
				tx,
				"email_challenges",
				"SELECT email,password_hash,code_hash,expires_at,attempts FROM email_challenges WHERE user_id = ? AND id = ?",
				challengeRow,
				user,
				id,
			)
			if err != nil {
				return webapiproto.UserDTO{}, err
			}
			if !ok {
				return webapiproto.UserDTO{}, ErrInvalidCode
			}
			var current string
			err = tx.QueryRowContext(ctx, "SELECT password_hash FROM users WHERE id = ?", user).Scan(&current)
			if err != nil && !errors.Is(err, sql.ErrNoRows) {
				return webapiproto.UserDTO{}, err
			}
			if errors.Is(err, sql.ErrNoRows) || found.expires <= now || found.attempts >= uint64(attempts) ||
				current != found.password {
				return webapiproto.UserDTO{}, ErrInvalidCode
			}
			if !codeHash.Matches(found.code) {
				refusal = ErrInvalidCode
				return webapiproto.UserDTO{}, execSQL(
					ctx,
					tx,
					"UPDATE email_challenges SET attempts = attempts + 1 WHERE user_id = ?",
					user,
				)
			}
			return consumeEmailChallenge(ctx, tx, user, found.email)
		},
	)
	if err != nil {
		return webapiproto.UserDTO{}, err
	}
	return changed, refusal
}

const userColumns = "id, email, nickname, role, created_at"

func userRow(r *storedRow) webapiproto.UserDTO {
	u := webapiproto.UserDTO{
		ID:        checked(r, "id", webapiproto.ParseUserID),
		Email:     checked(r, "email", webapiproto.ParseEmailAddress),
		Nickname:  r.text("nickname"),
		Role:      webapiproto.Role(r.text("role")),
		CreatedAt: r.instant("created_at"),
	}
	r.bad("role", u.Role.Validate())
	return u
}

func accountRow(r *storedRow) Account {
	u := userRow(r)
	hash, err := ParsePasswordHash(r.text("password_hash"))
	r.bad("password_hash", err)
	return Account{User: u, PasswordHash: hash}
}

func controlDo(
	ctx context.Context,
	c *ControlService,
	work func(context.Context, *sql.Tx, types.Timestamp) error,
) error {
	_, err := controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (struct{}, error) {
		return struct{}{}, work(ctx, tx, now)
	})
	return err
}

func savedPreferences(ctx context.Context, tx *sql.Tx, user webapiproto.UserID) (webapiproto.Preferences, error) {
	row, found, err := queryRecord(
		ctx,
		tx,
		"user_preferences",
		"SELECT preferences FROM user_preferences WHERE user_id = ?",
		func(r *storedRow) webapiproto.Preferences {
			return storedJSON(r, "preferences", webapiproto.DecodePreferences)
		},
		user,
	)
	if !found {
		return webapiproto.Preferences{}, err
	}
	return row, err
}

// mergePreferences preserves absent overrides and applies explicit shortcut nulls.
func mergePreferences(p *webapiproto.Preferences, patch webapiproto.PreferencesPatch) {
	if a := patch.Appearance; a != nil {
		if a.Theme != nil {
			p.Appearance.Theme = a.Theme
		}
		if a.Tone != nil {
			p.Appearance.Tone = a.Tone
		}
		if a.Accent != nil {
			p.Appearance.Accent = a.Accent
		}
		if a.FontSize != nil {
			p.Appearance.FontSize = a.FontSize
		}
	}
	if s := patch.Shortcuts; s != nil {
		if s.New != nil {
			p.Shortcuts.New = *s.New
		}
		if s.Sidebar != nil {
			p.Shortcuts.Sidebar = *s.Sidebar
		}
		if s.Settings != nil {
			p.Shortcuts.Settings = *s.Settings
		}
	}
	if patch.LastModel != nil {
		p.LastModel = patch.LastModel
	}
	if patch.Locale != nil {
		p.Locale = patch.Locale
	}
}

type challenge struct {
	email          webapiproto.EmailAddress
	password, code string
	expires        types.Timestamp
	attempts       uint64
}

// challengeRow validates stored challenge fields before code verification.
func challengeRow(r *storedRow) challenge {
	count := r.count("attempts")
	if count > math.MaxUint32 {
		r.bad("attempts", fmt.Errorf("attempt count exceeds uint32"))
	}
	return challenge{
		checked(r, "email", webapiproto.ParseEmailAddress),
		r.text("password_hash"),
		r.text("code_hash"),
		r.instant("expires_at"),
		count,
	}
}

func consumeEmailChallenge(
	ctx context.Context,
	tx *sql.Tx,
	user webapiproto.UserID,
	email webapiproto.EmailAddress,
) (webapiproto.UserDTO, error) {
	var taken bool
	if err := tx.QueryRowContext(ctx, "SELECT EXISTS (SELECT 1 FROM users WHERE email = ? AND id != ?)", email, user).
		Scan(&taken); err != nil {
		return webapiproto.UserDTO{}, err
	}
	if taken {
		return webapiproto.UserDTO{}, ErrEmailTaken
	}
	changed, found, err := queryRecord(
		ctx,
		tx,
		"users",
		"UPDATE users SET email = ? WHERE id = ? RETURNING "+userColumns,
		userRow,
		email,
		user,
	)
	if err != nil {
		return webapiproto.UserDTO{}, err
	}
	if !found {
		return webapiproto.UserDTO{}, ErrInvalidCode
	}
	if err := execSQL(ctx, tx, "DELETE FROM email_challenges WHERE user_id = ?", user); err != nil {
		return webapiproto.UserDTO{}, err
	}
	return changed, nil
}

func emailChallengeCooling(sent int64, now types.Timestamp, policy ChallengePolicy) (bool, error) {
	stamp, err := types.TimestampFromMillisecond(sent)
	if err != nil {
		return false, CorruptValue("email_challenges", "sent_at", err)
	}
	sentTime, err := stamp.Time()
	if err != nil {
		return false, err
	}
	current, err := now.Time()
	if err != nil {
		return false, err
	}
	return current.Sub(sentTime) < policy.Cooldown, nil
}

// ErrAlreadySetUp means the instance already has its master account.
var ErrAlreadySetUp = errors.New("the instance already has its master account")

// ErrUserNotFound means no account has the ID.
var ErrUserNotFound = errors.New("no such account")
