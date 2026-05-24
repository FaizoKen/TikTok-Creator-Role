//! Server-to-server client for the centralized Auth Gateway's
//! `/auth/internal/*` endpoints.
//!
//! All errors are bubbled up — callers (sync workers) should log and skip
//! the affected user/role-link this cycle, NOT silently treat the failure
//! as "no guilds" (Convention 40 — that would clear roles incorrectly).

use serde::Deserialize;

use crate::error::AppError;

/// Plugin slug sent to the Auth Gateway. Must match the URL prefix this
/// plugin is mounted under (`/tiktok-creator-role`) and the entry in the
/// gateway's plugin registry. The gateway uses this to scope the user's
/// per-(plugin × server) opt-outs when filtering guild lists.
const PLUGIN_SLUG: &str = "tiktok-creator-role";

#[derive(Debug, Deserialize)]
struct UserGuildIdsResponse {
    guild_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GuildMemberIdsResponse {
    discord_ids: Vec<String>,
}

pub async fn fetch_user_guild_ids(
    http: &reqwest::Client,
    base: &str,
    key: &str,
    discord_id: &str,
) -> Result<Vec<String>, AppError> {
    let url = format!("{base}/auth/internal/user_guild_ids");
    let resp = http
        .get(&url)
        .header("X-Internal-Key", key)
        // `plugin` scopes the response to this plugin's opt-out preferences
        // so guilds where the user disabled this plugin are excluded.
        .query(&[("discord_id", discord_id), ("plugin", PLUGIN_SLUG)])
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("auth_gateway request failed: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Internal(format!(
            "auth_gateway user_guild_ids returned {status}: {body}"
        )));
    }

    let parsed: UserGuildIdsResponse = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("auth_gateway response not JSON: {e}")))?;
    Ok(parsed.guild_ids)
}

pub async fn fetch_guild_member_ids(
    http: &reqwest::Client,
    base: &str,
    key: &str,
    guild_id: &str,
) -> Result<Vec<String>, AppError> {
    let url = format!("{base}/auth/internal/guild_member_ids");
    let resp = http
        .get(&url)
        .header("X-Internal-Key", key)
        // `plugin` excludes members who have opted out of this plugin in
        // this guild, so the atomic role replacement on the next sync
        // drops their role on Discord's side too.
        .query(&[("guild_id", guild_id), ("plugin", PLUGIN_SLUG)])
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("auth_gateway request failed: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Internal(format!(
            "auth_gateway guild_member_ids returned {status}: {body}"
        )));
    }

    let parsed: GuildMemberIdsResponse = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("auth_gateway response not JSON: {e}")))?;
    Ok(parsed.discord_ids)
}
