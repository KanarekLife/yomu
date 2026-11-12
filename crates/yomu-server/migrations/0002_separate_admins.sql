-- Panel admins are no longer app users (they sign in via OIDC or the env
-- break-glass login). Drop admin rows that own nothing (bootstrap leftovers);
-- admins with data stay as ordinary users.
DELETE FROM users u
WHERE u.is_admin
  AND NOT EXISTS (SELECT 1 FROM books b WHERE b.user_id = u.id)
  AND NOT EXISTS (SELECT 1 FROM settings s WHERE s.user_id = u.id)
  AND NOT EXISTS (SELECT 1 FROM translations t WHERE t.user_id = u.id)
  AND NOT EXISTS (SELECT 1 FROM files f WHERE f.user_id = u.id);

ALTER TABLE users DROP COLUMN is_admin;

-- Sessions now name an admin identity instead of a user row.
DROP TABLE admin_sessions;
CREATE TABLE admin_sessions (
    id_hash    BYTEA PRIMARY KEY,
    admin      TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);
