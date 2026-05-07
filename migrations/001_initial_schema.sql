-- Per-RoleLogic-role-link configuration
CREATE TABLE IF NOT EXISTS role_links (
    id              BIGSERIAL PRIMARY KEY,
    guild_id        TEXT        NOT NULL,
    role_id         TEXT        NOT NULL,
    api_token       TEXT        NOT NULL,
    conditions      JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (guild_id, role_id)
);

-- Discord <-> TikTok identity link, plus user OAuth tokens
CREATE TABLE IF NOT EXISTS linked_accounts (
    id                          BIGSERIAL PRIMARY KEY,
    discord_id                  TEXT        NOT NULL UNIQUE,
    tiktok_open_id              TEXT        NOT NULL UNIQUE,
    tiktok_union_id             TEXT,
    tiktok_username             TEXT        NOT NULL,
    tiktok_display_name         TEXT,
    tiktok_avatar_url           TEXT,
    tiktok_access_token         TEXT        NOT NULL,
    tiktok_refresh_token        TEXT        NOT NULL,
    tiktok_token_expires_at     TIMESTAMPTZ NOT NULL,
    tiktok_refresh_expires_at   TIMESTAMPTZ NOT NULL,
    tiktok_scope                TEXT        NOT NULL,
    linked_at                   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at                  TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS linked_accounts_active_idx
    ON linked_accounts(discord_id) WHERE revoked_at IS NULL;

-- Cached Display API stats — one row per linked Discord user.
-- Denormalized for SQL-side filtering during config_sync (Convention 6, 8).
CREATE TABLE IF NOT EXISTS tiktok_stats_cache (
    discord_id          TEXT        PRIMARY KEY,
    is_verified         BOOLEAN     NOT NULL DEFAULT FALSE,
    follower_count      BIGINT      NOT NULL DEFAULT 0,
    following_count     BIGINT      NOT NULL DEFAULT 0,
    likes_count         BIGINT      NOT NULL DEFAULT 0,
    video_count         BIGINT      NOT NULL DEFAULT 0,
    has_bio             BOOLEAN     NOT NULL DEFAULT FALSE,
    bio_length          INTEGER     NOT NULL DEFAULT 0,
    fetched_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    next_fetch_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    fetch_failures      INTEGER     NOT NULL DEFAULT 0,
    last_error          TEXT
);
CREATE INDEX IF NOT EXISTS stats_next_fetch_idx ON tiktok_stats_cache(next_fetch_at);

-- Local mirror of role grants (cascade-deletes when role link is removed)
CREATE TABLE IF NOT EXISTS role_assignments (
    guild_id        TEXT        NOT NULL,
    role_id         TEXT        NOT NULL,
    discord_id      TEXT        NOT NULL,
    assigned_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (guild_id, role_id, discord_id),
    FOREIGN KEY (guild_id, role_id) REFERENCES role_links(guild_id, role_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS role_assignments_discord_idx ON role_assignments(discord_id);

-- CSRF state for the plugin's own TikTok OAuth flow
-- (Convention 37 carve-out: a plugin's secondary OAuth gets its own oauth_states table.)
CREATE TABLE IF NOT EXISTS oauth_states (
    state           TEXT        PRIMARY KEY,
    discord_id      TEXT        NOT NULL,
    expires_at      TIMESTAMPTZ NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS oauth_states_expires_idx ON oauth_states(expires_at);
