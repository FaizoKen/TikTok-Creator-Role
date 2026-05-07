# TikTok Creator Role

RoleLogic plugin that gates Discord roles by a member's TikTok account stats — verified status, follower count, video count, total likes, etc.

The plugin uses TikTok Login Kit (OAuth 2.0) and the Display API (`/v2/user/info/`). It does **not** verify "follows my channel" — TikTok's public API does not expose follower-list checks for third-party apps.

## What admins can require

- Verified account (blue check)
- Minimum follower count (e.g. 10k+ "Influencer", 100k+ "Mega creator")
- Minimum following count
- Minimum total likes
- Minimum video count
- Non-empty bio of a minimum length

All conditions are AND'd together. Leave all toggles off to grant the role to anyone who links a TikTok account.

## Architecture

Standard RoleLogic plugin shape — see `.claude/BLUEPRINT.md` at the repo root.

- Rust 1.88 / Axum 0.8 / SQLx + Postgres / Tokio
- Refresh worker uses adaptive polling: interval scales with cache size against `TIKTOK_QUOTA_PER_DAY`, active users prioritized, exponential backoff on failures.
- Player sync (event-driven) and config sync (5-second debounced) workers mirror the Twitch / YouTube plugin patterns.
- Discord OAuth and `user_guilds`/`discord_tokens` live in the centralized Auth Gateway, never in this plugin's DB.

## Local dev

```sh
cp .env.example .env
# fill in TIKTOK_CLIENT_KEY, TIKTOK_CLIENT_SECRET, SESSION_SECRET, INTERNAL_API_KEY
docker compose up -d db
cargo run
```

In a separate terminal, point Cloudflare Tunnel (or a reverse proxy) at `localhost:8088` under `/tiktok-creator-role`.

## TikTok app setup

1. Create an app at https://developers.tiktok.com/apps
2. Enable Login Kit; request scopes `user.info.basic,user.info.profile,user.info.stats`
3. Add redirect URI `${BASE_URL}/verify/tiktok/callback`

## RoleLogic dashboard registration

Register the plugin URL `https://plugin-rolelogic.faizo.net/tiktok-creator-role`.
