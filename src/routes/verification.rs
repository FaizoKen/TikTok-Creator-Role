use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar};
use rand::Rng;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::services::session;
use crate::services::sync::PlayerSyncEvent;
use crate::services::tiktok::TikTokUserInfo;
use crate::AppState;

const SESSION_COOKIE: &str = "rl_session";

fn get_session(jar: &CookieJar, secret: &str) -> Result<(String, String), AppError> {
    let cookie = jar.get(SESSION_COOKIE).ok_or_else(|| {
        AppError::UnauthorizedWith("Missing rl_session cookie — please sign in with Discord".into())
    })?;
    session::verify_session(cookie.value(), secret).ok_or_else(|| {
        AppError::UnauthorizedWith(
            "Session cookie invalid or expired — please sign in with Discord again".into(),
        )
    })
}

fn random_state() -> String {
    let mut rng = rand::thread_rng();
    (0..32)
        .map(|_| {
            let idx = rng.gen_range(0..36);
            if idx < 10 {
                (b'0' + idx) as char
            } else {
                (b'a' + idx - 10) as char
            }
        })
        .collect()
}

pub fn render_verify_page(base_url: &str) -> String {
    format!(
        r##"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>TikTok Creator Role - Link Account</title>
    <link rel="icon" href="{base_url}/favicon.ico" type="image/x-icon">
    <meta name="description" content="Link your Discord and TikTok accounts to automatically receive server roles based on your creator profile.">
    <meta name="theme-color" content="#FE2C55">
    <style>
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{ font-family: system-ui, -apple-system, sans-serif; max-width: 580px; margin: 0 auto; padding: 32px 20px; background: #010101; color: #c8ccd4; min-height: 100vh; }}
        h1 {{ background: linear-gradient(90deg,#FE2C55,#25F4EE); -webkit-background-clip:text; background-clip:text; color: transparent; font-size: 26px; margin-bottom: 4px; font-weight: 800; letter-spacing: -.5px; }}
        p {{ line-height: 1.6; margin: 6px 0; font-size: 14px; }}
        a {{ color: #25F4EE; }}
        .subtitle {{ color: #7a8299; font-size: 14px; margin-bottom: 20px; }}
        .card {{ background: #111114; padding: 22px; border-radius: 12px; margin: 14px 0; border: 1px solid #25272d; }}
        .btn {{ display: inline-flex; align-items: center; gap: 8px; padding: 10px 22px; color: #fff; text-decoration: none; border-radius: 8px; font-size: 14px; font-weight: 600; border: none; cursor: pointer; font-family: inherit; transition: filter .15s, transform .05s; }}
        .btn:active {{ transform: translateY(1px); }}
        .btn-discord {{ background: #5865f2; }}
        .btn-discord:hover {{ filter: brightness(1.1); }}
        .btn-tiktok {{ background: linear-gradient(90deg,#FE2C55,#25F4EE); color: #010101; }}
        .btn-tiktok:hover {{ filter: brightness(1.1); }}
        .btn-danger {{ background: transparent; color: #f87171; border: 1px solid #7f1d1d; font-size: 13px; padding: 8px 16px; }}
        .btn-danger:hover {{ background: #7f1d1d33; }}
        .btn-secondary {{ background: transparent; color: #94a3b8; border: 1px solid #334155; font-size: 13px; padding: 8px 16px; }}
        .btn-secondary:hover {{ background: #1e293b; }}
        .badge {{ display: inline-block; padding: 3px 10px; border-radius: 20px; font-size: 12px; font-weight: 500; }}
        .badge-ok {{ background: #052e16; color: #4ade80; border: 1px solid #14532d; }}
        .badge-tiktok {{ background: #1a0a0f; color: #FE2C55; border: 1px solid #7f1d1d; }}
        .msg {{ padding: 10px 14px; border-radius: 6px; margin: 12px 0; font-size: 13px; line-height: 1.5; }}
        .msg-error {{ background: #1c0a0a; color: #fca5a5; border: 1px solid #7f1d1d; }}
        .hidden {{ display: none; }}
        .actions {{ display: flex; gap: 10px; margin-top: 14px; flex-wrap: wrap; }}
        .stats {{ display: grid; grid-template-columns: repeat(2, 1fr); gap: 10px; margin: 14px 0 4px; }}
        .stat {{ background: #0a0b0d; border: 1px solid #25272d; border-radius: 8px; padding: 10px 12px; }}
        .stat-label {{ color: #7a8299; font-size: 11px; text-transform: uppercase; letter-spacing: .5px; }}
        .stat-value {{ color: #f1f5f9; font-size: 18px; font-weight: 600; margin-top: 2px; }}
        .verified {{ color: #25F4EE; font-weight: 600; }}
        .username {{ color: #FE2C55; font-weight: 600; }}
        .guild-ctx {{ display: none; align-items: center; gap: 10px; background: #052e16; border: 1px solid #14532d; color: #86efac; padding: 8px 14px; border-radius: 8px; margin: 12px 0 6px; font-size: 13px; line-height: 1.5; }}
        .guild-ctx.show {{ display: flex; }}
        .guild-ctx.warn {{ background: #1c1208; border-color: #422006; color: #fbbf24; }}
        .guild-ctx .gctx-icon {{ flex-shrink: 0; }}
        .guild-ctx .gctx-name {{ color: #fff; font-weight: 600; }}
        .manage-servers {{ font-size: 13px; color: #94a3b8; margin-top: 14px; line-height: 1.6; }}
        .manage-servers a {{ color: #25F4EE; }}
    </style>
</head>
<body>
    <h1>TikTok Creator Role</h1>
    <p class="subtitle">Link your Discord and TikTok accounts</p>

    <!-- Server context banner: only shown when ?guild=<id> is present in the URL.
         Lets a server admin share a per-guild link that both verifies the user
         AND auto-enables the role for that specific server in one shot. -->
    <div id="guild-ctx" class="guild-ctx">
        <svg class="gctx-icon" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
        <span id="guild-ctx-text"></span>
    </div>

    <div id="loading" class="card"><p>Loading...</p></div>

    <div id="login" class="card hidden">
        <p>Sign in with Discord to get started.</p>
        <div class="actions">
            <a href="{base_url}/verify/login" class="btn btn-discord">Login with Discord</a>
        </div>
    </div>

    <div id="link-tiktok" class="card hidden">
        <p>Logged in as <strong id="discord-name"></strong> <span class="badge badge-ok">Discord</span></p>
        <p style="margin-top:12px;">Now link your TikTok account so we can verify your creator stats:</p>
        <div class="actions">
            <a href="{base_url}/verify/tiktok" class="btn btn-tiktok">Link TikTok Account</a>
            <button onclick="doLogout()" class="btn btn-secondary">Logout</button>
        </div>
    </div>

    <div id="linked" class="card hidden">
        <p>Logged in as <strong id="discord-name2"></strong> <span class="badge badge-ok">Discord</span></p>
        <p>TikTok: <span class="username" id="tiktok-name"></span> <span class="badge badge-tiktok">Linked</span> <span id="verified-badge" class="hidden"><span class="verified">✓ Verified</span></span></p>
        <div class="stats">
            <div class="stat"><div class="stat-label">Followers</div><div class="stat-value" id="stat-followers">—</div></div>
            <div class="stat"><div class="stat-label">Following</div><div class="stat-value" id="stat-following">—</div></div>
            <div class="stat"><div class="stat-label">Total Likes</div><div class="stat-value" id="stat-likes">—</div></div>
            <div class="stat"><div class="stat-label">Videos</div><div class="stat-value" id="stat-videos">—</div></div>
        </div>
        <p style="margin-top:8px; color:#86efac; font-size:13px;">Your accounts are linked. Roles will be assigned automatically based on your TikTok creator stats.</p>
        <p class="manage-servers">
            Receiving TikTok roles in servers you didn't intend?
            <a href="/auth/my_servers?from=/tiktok-creator-role/verify">Choose which servers receive roles →</a>
        </p>
        <div class="actions">
            <button onclick="doUnlink()" class="btn btn-danger">Unlink TikTok</button>
            <button onclick="doLogout()" class="btn btn-secondary">Logout</button>
        </div>
    </div>

    <div id="error" class="msg msg-error hidden"></div>

    <script>
    const PLUGIN_SLUG = 'tiktok-creator-role';

    // Optional ?guild=<id> tells us the user came from a per-guild verify
    // link an admin shared in their Discord. We use it to (a) show a
    // contextual banner so the user knows which server this is for and
    // (b) automatically clear any existing opt-out (both per-plugin and
    // the guild-wide master) once they're authenticated — so a returning
    // user who'd previously disabled this server doesn't have to find
    // /auth/my_servers to re-enable it.
    const guildId = (() => {{
        try {{
            const v = new URLSearchParams(window.location.search).get('guild');
            return v && /^[0-9]{{5,25}}$/.test(v) ? v : '';
        }} catch (e) {{ return ''; }}
    }})();

    // Preserve the guild context across the Discord OAuth round-trip so
    // an unauth visitor who logs in lands back on this same per-guild URL.
    (function patchLoginHref() {{
        if (!guildId) return;
        const link = document.querySelector('#login a.btn-discord');
        if (!link) return;
        const returnTo = '/tiktok-creator-role/verify?guild=' + encodeURIComponent(guildId);
        link.href = '/auth/login?return_to=' + encodeURIComponent(returnTo);
    }})();

    // Gateway-absolute API helper for /auth/* (cookie-authed via the
    // shared rl_session).
    async function gatewayApi(method, path, body) {{
        const opts = {{ method, headers: {{}}, credentials: 'include' }};
        if (body) {{
            opts.headers['Content-Type'] = 'application/json';
            opts.body = JSON.stringify(body);
        }}
        const res = await fetch(path, opts);
        const data = await res.json().catch(() => ({{}}));
        if (!res.ok) throw new Error(data.error || 'Request failed');
        return data;
    }}

    function showGuildCtx(text, isWarning) {{
        const el = document.getElementById('guild-ctx');
        document.getElementById('guild-ctx-text').innerHTML = text;
        el.classList.toggle('warn', !!isWarning);
        el.classList.add('show');
    }}

    let isLinked = false;

    // Resolve guildId → display name via the gateway, then clear any
    // opt-out blocking this plugin from assigning roles in that server.
    // Idempotent: clearing rows that don't exist is a no-op on the server.
    async function applyGuildContext() {{
        if (!guildId) return;
        let prefs;
        try {{
            prefs = await gatewayApi('GET', '/auth/preferences?ensure_guild=' + encodeURIComponent(guildId));
        }} catch (e) {{
            return;
        }}
        const g = (prefs.guilds || []).find(x => x.guild_id === guildId);
        if (!g) {{
            showGuildCtx("You're not in that server yet — join it on Discord, then refresh.", true);
            return;
        }}
        const safeName = (g.guild_name || '(unnamed server)')
            .replace(/[&<>"']/g, c => ({{'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}})[c]);
        const wasDisabled = g.master_optout || (g.plugin_optouts || []).includes(PLUGIN_SLUG);
        try {{
            if (g.master_optout) {{
                await gatewayApi('POST', '/auth/preferences', {{
                    guild_id: guildId, plugin: null, enabled: true,
                }});
            }}
            if ((g.plugin_optouts || []).includes(PLUGIN_SLUG)) {{
                await gatewayApi('POST', '/auth/preferences', {{
                    guild_id: guildId, plugin: PLUGIN_SLUG, enabled: true,
                }});
            }}
        }} catch (e) {{
            // Even if the clear failed, still show the banner so the user
            // knows where they are. They can fix it via /auth/my_servers.
        }}
        const nameHtml = '<span class="gctx-name">' + safeName + '</span>';
        if (wasDisabled) {{
            showGuildCtx(isLinked
                ? 'Enabled TikTok roles for ' + nameHtml + ' — roles apply on the next sync.'
                : 'Enabled TikTok roles for ' + nameHtml + ' — finish linking below to receive roles.');
        }} else {{
            showGuildCtx(isLinked
                ? 'TikTok roles are active in ' + nameHtml + '.'
                : 'Once linked, TikTok roles will apply in ' + nameHtml + '.');
        }}
    }}

    function fmt(n) {{
        if (n === null || n === undefined) return '—';
        if (n >= 1e9) return (n / 1e9).toFixed(1) + 'B';
        if (n >= 1e6) return (n / 1e6).toFixed(1) + 'M';
        if (n >= 1e3) return (n / 1e3).toFixed(1) + 'K';
        return String(n);
    }}
    async function init() {{
        try {{
            const r = await fetch('{base_url}/verify/status', {{credentials:'include'}});
            const d = await r.json();
            document.getElementById('loading').classList.add('hidden');
            if (!d.discord_id) {{
                document.getElementById('login').classList.remove('hidden');
                if (d.error) showError(d.error);
            }} else if (!d.tiktok) {{
                document.getElementById('discord-name').textContent = d.discord_name;
                document.getElementById('link-tiktok').classList.remove('hidden');
                applyGuildContext();
            }} else {{
                document.getElementById('discord-name2').textContent = d.discord_name;
                document.getElementById('tiktok-name').textContent = '@' + d.tiktok.username;
                document.getElementById('stat-followers').textContent = fmt(d.tiktok.follower_count);
                document.getElementById('stat-following').textContent = fmt(d.tiktok.following_count);
                document.getElementById('stat-likes').textContent = fmt(d.tiktok.likes_count);
                document.getElementById('stat-videos').textContent = fmt(d.tiktok.video_count);
                if (d.tiktok.is_verified) document.getElementById('verified-badge').classList.remove('hidden');
                document.getElementById('linked').classList.remove('hidden');
                isLinked = true;
                applyGuildContext();
            }}
        }} catch(e) {{
            document.getElementById('loading').classList.add('hidden');
            document.getElementById('login').classList.remove('hidden');
            showError('Failed to load status. Try refreshing.');
        }}
    }}
    async function doUnlink() {{
        if (!confirm('Unlink your TikTok account? You will lose all roles assigned by this plugin.')) return;
        const r = await fetch('{base_url}/verify/unlink', {{method:'POST', credentials:'include'}});
        if (r.ok) location.reload();
        else {{ const d = await r.json(); showError(d.error || 'Unlink failed'); }}
    }}
    async function doLogout() {{
        await fetch('{base_url}/verify/logout', {{method:'POST', credentials:'include'}});
        location.reload();
    }}
    function showError(msg) {{
        const el = document.getElementById('error');
        el.textContent = msg;
        el.classList.remove('hidden');
    }}
    init();
    </script>
</body>
</html>"##
    )
}

pub async fn verify_page(State(state): State<Arc<AppState>>) -> Response {
    (
        StatusCode::OK,
        [("content-type", "text/html; charset=utf-8")],
        state.verify_html.clone(),
    )
        .into_response()
}

pub async fn login() -> Redirect {
    let return_to = "/tiktok-creator-role/verify";
    let url = format!("/auth/login?return_to={}", urlencoding::encode(return_to));
    Redirect::temporary(&url)
}

pub async fn status(State(state): State<Arc<AppState>>, jar: CookieJar) -> Json<Value> {
    let session = get_session(&jar, &state.config.session_secret);

    match session {
        Ok((discord_id, display_name)) => {
            let row = sqlx::query_as::<_, (String, bool, i64, i64, i64, i64)>(
                "SELECT la.tiktok_username, COALESCE(c.is_verified, FALSE), \
                        COALESCE(c.follower_count, 0), COALESCE(c.following_count, 0), \
                        COALESCE(c.likes_count, 0), COALESCE(c.video_count, 0) \
                 FROM linked_accounts la \
                 LEFT JOIN tiktok_stats_cache c ON c.discord_id = la.discord_id \
                 WHERE la.discord_id = $1 AND la.revoked_at IS NULL",
            )
            .bind(&discord_id)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten();

            let tiktok = row.map(|(username, is_verified, fc, fg, lc, vc)| {
                json!({
                    "username": username,
                    "is_verified": is_verified,
                    "follower_count": fc,
                    "following_count": fg,
                    "likes_count": lc,
                    "video_count": vc,
                })
            });

            Json(json!({
                "discord_id": discord_id,
                "discord_name": display_name,
                "tiktok": tiktok,
            }))
        }
        Err(_) => Json(json!({
            "discord_id": null,
            "discord_name": null,
            "tiktok": null,
        })),
    }
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: String,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

pub async fn tiktok_start(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Redirect, AppError> {
    let (discord_id, _) = get_session(&jar, &state.config.session_secret)?;

    let state_token = random_state();
    sqlx::query(
        "INSERT INTO oauth_states (state, discord_id, expires_at) \
         VALUES ($1, $2, now() + interval '10 minutes')",
    )
    .bind(&state_token)
    .bind(&discord_id)
    .execute(&state.pool)
    .await?;

    let redirect_uri = state.config.tiktok_oauth_redirect_uri();
    let url = state
        .tiktok_client
        .build_authorize_url(&redirect_uri, &state_token);

    Ok(Redirect::temporary(&url))
}

pub async fn tiktok_callback(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CallbackQuery>,
) -> Result<Redirect, AppError> {
    if let Some(err) = query.error {
        let desc = query.error_description.unwrap_or_default();
        return Err(AppError::BadRequest(format!(
            "TikTok OAuth was cancelled or failed: {err} - {desc}"
        )));
    }
    let code = query.code.ok_or_else(|| {
        AppError::BadRequest("Missing authorization code in TikTok callback".into())
    })?;

    // Validate state and recover discord_id.
    let row = sqlx::query_as::<_, (String,)>(
        "DELETE FROM oauth_states WHERE state = $1 AND expires_at > now() RETURNING discord_id",
    )
    .bind(&query.state)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::BadRequest("Invalid or expired OAuth state".into()))?;

    let discord_id = row.0;

    let redirect_uri = state.config.tiktok_oauth_redirect_uri();
    let tokens = state
        .tiktok_client
        .exchange_code(&code, &redirect_uri)
        .await?;

    let user_info: TikTokUserInfo = state
        .tiktok_client
        .fetch_user_info(&tokens.access_token)
        .await?;

    // Make sure this TikTok account isn't already linked to a *different* Discord user.
    let existing = sqlx::query_scalar::<_, String>(
        "SELECT discord_id FROM linked_accounts WHERE tiktok_open_id = $1 AND revoked_at IS NULL",
    )
    .bind(&user_info.open_id)
    .fetch_optional(&state.pool)
    .await?;

    if let Some(existing_discord_id) = existing {
        if existing_discord_id != discord_id {
            return Err(AppError::BadRequest(
                "This TikTok account is already linked to another Discord user".into(),
            ));
        }
    }

    let token_expires_at = chrono::Utc::now() + chrono::Duration::seconds(tokens.expires_in);
    let refresh_expires_at =
        chrono::Utc::now() + chrono::Duration::seconds(tokens.refresh_expires_in);

    // Upsert the linked account by discord_id (a Discord user can re-link to the same or
    // a different TikTok account; we replace the row either way).
    sqlx::query(
        "INSERT INTO linked_accounts ( \
            discord_id, tiktok_open_id, tiktok_union_id, tiktok_username, tiktok_display_name, \
            tiktok_avatar_url, tiktok_access_token, tiktok_refresh_token, \
            tiktok_token_expires_at, tiktok_refresh_expires_at, tiktok_scope, linked_at, revoked_at \
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, now(), NULL) \
         ON CONFLICT (discord_id) DO UPDATE SET \
            tiktok_open_id = EXCLUDED.tiktok_open_id, \
            tiktok_union_id = EXCLUDED.tiktok_union_id, \
            tiktok_username = EXCLUDED.tiktok_username, \
            tiktok_display_name = EXCLUDED.tiktok_display_name, \
            tiktok_avatar_url = EXCLUDED.tiktok_avatar_url, \
            tiktok_access_token = EXCLUDED.tiktok_access_token, \
            tiktok_refresh_token = EXCLUDED.tiktok_refresh_token, \
            tiktok_token_expires_at = EXCLUDED.tiktok_token_expires_at, \
            tiktok_refresh_expires_at = EXCLUDED.tiktok_refresh_expires_at, \
            tiktok_scope = EXCLUDED.tiktok_scope, \
            linked_at = now(), \
            revoked_at = NULL",
    )
    .bind(&discord_id)
    .bind(&user_info.open_id)
    .bind(&user_info.union_id)
    .bind(&user_info.username)
    .bind(&user_info.display_name)
    .bind(&user_info.avatar_url)
    .bind(&tokens.access_token)
    .bind(&tokens.refresh_token)
    .bind(token_expires_at)
    .bind(refresh_expires_at)
    .bind(&tokens.scope)
    .execute(&state.pool)
    .await?;

    // Seed the stats cache row with the values we just fetched.
    let bio = user_info.bio_description.trim();
    let has_bio = !bio.is_empty();
    let bio_length = bio.chars().count() as i32;

    sqlx::query(
        "INSERT INTO tiktok_stats_cache ( \
            discord_id, is_verified, follower_count, following_count, \
            likes_count, video_count, has_bio, bio_length, \
            fetched_at, next_fetch_at, fetch_failures, last_error \
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now(), now() + interval '30 minutes', 0, NULL) \
         ON CONFLICT (discord_id) DO UPDATE SET \
            is_verified = EXCLUDED.is_verified, \
            follower_count = EXCLUDED.follower_count, \
            following_count = EXCLUDED.following_count, \
            likes_count = EXCLUDED.likes_count, \
            video_count = EXCLUDED.video_count, \
            has_bio = EXCLUDED.has_bio, \
            bio_length = EXCLUDED.bio_length, \
            fetched_at = now(), \
            next_fetch_at = now() + interval '30 minutes', \
            fetch_failures = 0, \
            last_error = NULL",
    )
    .bind(&discord_id)
    .bind(user_info.is_verified)
    .bind(user_info.follower_count)
    .bind(user_info.following_count)
    .bind(user_info.likes_count)
    .bind(user_info.video_count)
    .bind(has_bio)
    .bind(bio_length)
    .execute(&state.pool)
    .await?;

    let _ = state
        .player_sync_tx
        .send(PlayerSyncEvent::AccountLinked {
            discord_id: discord_id.clone(),
        })
        .await;

    tracing::info!(
        discord_id,
        tiktok_open_id = user_info.open_id,
        tiktok_username = user_info.username,
        is_verified = user_info.is_verified,
        follower_count = user_info.follower_count,
        "TikTok account linked"
    );

    Ok(Redirect::temporary("/tiktok-creator-role/verify"))
}

pub async fn unlink(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<Value>, AppError> {
    let (discord_id, _) = get_session(&jar, &state.config.session_secret)?;

    // Best-effort revoke the access token, then drop the local row.
    let access = sqlx::query_scalar::<_, String>(
        "SELECT tiktok_access_token FROM linked_accounts WHERE discord_id = $1",
    )
    .bind(&discord_id)
    .fetch_optional(&state.pool)
    .await?;

    if let Some(token) = access {
        state.tiktok_client.revoke_token(&token).await;
    }

    let deleted = sqlx::query("DELETE FROM linked_accounts WHERE discord_id = $1")
        .bind(&discord_id)
        .execute(&state.pool)
        .await?;

    sqlx::query("DELETE FROM tiktok_stats_cache WHERE discord_id = $1")
        .bind(&discord_id)
        .execute(&state.pool)
        .await
        .ok();

    if deleted.rows_affected() > 0 {
        let _ = state
            .player_sync_tx
            .send(PlayerSyncEvent::AccountUnlinked {
                discord_id: discord_id.clone(),
            })
            .await;

        tracing::info!(discord_id, "TikTok account unlinked");
    }

    Ok(Json(json!({"success": true})))
}

pub async fn logout(jar: CookieJar) -> (CookieJar, Json<Value>) {
    let cookie = Cookie::build((SESSION_COOKIE, ""))
        .path("/")
        .max_age(time::Duration::ZERO)
        .build();

    (jar.remove(cookie), Json(json!({"success": true})))
}
