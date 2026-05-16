use std::collections::HashSet;

use futures_util::stream::{self, StreamExt};
use sqlx::PgPool;

use crate::error::AppError;
use crate::models::condition::TikTokConditions;
use crate::services::auth_gateway;
use crate::services::condition_eval::{evaluate, CacheData};
use crate::AppState;

/// Events sent to the player sync worker (lightweight, per-user).
#[derive(Debug, Clone)]
pub enum PlayerSyncEvent {
    AccountLinked { discord_id: String },
    AccountUnlinked { discord_id: String },
    PlayerUpdated { discord_id: String },
}

/// Events sent to the config sync worker (heavy, per-role-link).
#[derive(Debug, Clone)]
pub struct ConfigSyncEvent {
    pub guild_id: String,
    pub role_id: String,
}

/// Sync roles for a single user across all guilds they belong to.
/// Evaluates conditions locally, then executes RoleLogic API calls concurrently.
pub async fn sync_for_player(discord_id: &str, state: &AppState) -> Result<(), AppError> {
    let pool = &state.pool;
    let rl_client = &state.rl_client;

    // Make sure the account is linked and not revoked.
    let linked = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM linked_accounts WHERE discord_id = $1 AND revoked_at IS NULL)",
    )
    .bind(discord_id)
    .fetch_one(pool)
    .await
    .unwrap_or(false);

    if !linked {
        return Ok(());
    }

    // User's current stats.
    let cache = sqlx::query_as::<_, (bool, i64, i64, i64, i64, bool, i32)>(
        "SELECT is_verified, follower_count, following_count, likes_count, video_count, has_bio, bio_length \
         FROM tiktok_stats_cache WHERE discord_id = $1",
    )
    .bind(discord_id)
    .fetch_optional(pool)
    .await?;

    let cache_data = cache.map(|(iv, fc, fg, lc, vc, hb, bl)| CacheData {
        is_verified: iv,
        follower_count: fc,
        following_count: fg,
        likes_count: lc,
        video_count: vc,
        has_bio: hb,
        bio_length: bl,
    });

    // Pull guild IDs from Auth Gateway.
    let guild_ids = auth_gateway::fetch_user_guild_ids(
        &state.http,
        &state.config.auth_gateway_url,
        &state.config.internal_api_key,
        discord_id,
    )
    .await?;

    if guild_ids.is_empty() {
        return Ok(());
    }

    let role_links = sqlx::query_as::<_, (String, String, String, sqlx::types::Json<TikTokConditions>)>(
        "SELECT guild_id, role_id, api_token, conditions FROM role_links WHERE guild_id = ANY($1)",
    )
    .bind(&guild_ids[..])
    .fetch_all(pool)
    .await?;

    if role_links.is_empty() {
        return Ok(());
    }

    let existing: HashSet<(String, String)> = sqlx::query_as::<_, (String, String)>(
        "SELECT guild_id, role_id FROM role_assignments WHERE discord_id = $1",
    )
    .bind(discord_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect();

    enum Action {
        Add {
            guild_id: String,
            role_id: String,
            api_token: String,
        },
        Remove {
            guild_id: String,
            role_id: String,
            api_token: String,
        },
    }

    let mut actions: Vec<Action> = Vec::new();
    for (guild_id, role_id, api_token, conditions) in &role_links {
        let qualifies = match cache_data.as_ref() {
            Some(c) => evaluate(conditions, c),
            None => false, // No stats fetched yet → cannot qualify.
        };
        let currently_assigned = existing.contains(&(guild_id.clone(), role_id.clone()));
        match (qualifies, currently_assigned) {
            (true, false) => actions.push(Action::Add {
                guild_id: guild_id.clone(),
                role_id: role_id.clone(),
                api_token: api_token.clone(),
            }),
            (false, true) => actions.push(Action::Remove {
                guild_id: guild_id.clone(),
                role_id: role_id.clone(),
                api_token: api_token.clone(),
            }),
            _ => {}
        }
    }

    if actions.is_empty() {
        return Ok(());
    }

    let discord_id_owned = discord_id.to_string();
    stream::iter(actions)
        .for_each_concurrent(10, |action| {
            let pool = pool.clone();
            let rl_client = rl_client.clone();
            let discord_id = discord_id_owned.clone();
            async move {
                match action {
                    Action::Add { guild_id, role_id, api_token } => {
                        match rl_client.add_user(&guild_id, &role_id, &discord_id, &api_token).await {
                            Err(AppError::RoleLinkNotFound) => {
                                delete_orphan_role_link(&guild_id, &role_id, &pool).await;
                                return;
                            }
                            Err(AppError::UserLimitReached { limit }) => {
                                tracing::warn!(guild_id, role_id, discord_id, limit, "Cannot add user: role link user limit reached");
                                return;
                            }
                            Err(e) => {
                                tracing::error!(guild_id, role_id, discord_id, "Failed to add user to role: {e}");
                                return;
                            }
                            Ok(_) => {}
                        }
                        if let Err(e) = sqlx::query(
                            "INSERT INTO role_assignments (guild_id, role_id, discord_id) \
                             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
                        )
                        .bind(&guild_id)
                        .bind(&role_id)
                        .bind(&discord_id)
                        .execute(&pool)
                        .await
                        {
                            tracing::error!(guild_id, role_id, discord_id, "Failed to insert assignment: {e}");
                        }
                    }
                    Action::Remove { guild_id, role_id, api_token } => {
                        match rl_client.remove_user(&guild_id, &role_id, &discord_id, &api_token).await {
                            Err(AppError::RoleLinkNotFound) => {
                                delete_orphan_role_link(&guild_id, &role_id, &pool).await;
                                return;
                            }
                            Err(e) => {
                                tracing::error!(guild_id, role_id, discord_id, "Failed to remove user from role: {e}");
                                return;
                            }
                            Ok(_) => {}
                        }
                        if let Err(e) = sqlx::query(
                            "DELETE FROM role_assignments WHERE guild_id = $1 AND role_id = $2 AND discord_id = $3",
                        )
                        .bind(&guild_id)
                        .bind(&role_id)
                        .bind(&discord_id)
                        .execute(&pool)
                        .await
                        {
                            tracing::error!(guild_id, role_id, discord_id, "Failed to delete assignment: {e}");
                        }
                    }
                }
            }
        })
        .await;

    Ok(())
}

enum ConditionBind {
    Int(i64),
    SmallInt(i32),
}

/// Build a SQL WHERE clause + parametric binds against `tiktok_stats_cache` columns.
/// Returns ("TRUE", []) when no toggles are enabled — the role grants to anyone linked.
fn build_condition_where(conditions: &TikTokConditions) -> (String, Vec<ConditionBind>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut binds: Vec<ConditionBind> = Vec::new();

    if conditions.require_verified {
        clauses.push("c.is_verified = TRUE".to_string());
    }

    if conditions.require_followers {
        if conditions.min_followers > 0 {
            let idx = binds.len() + 1;
            clauses.push(format!("c.follower_count >= ${idx}"));
            binds.push(ConditionBind::Int(conditions.min_followers));
        }
    }

    if conditions.require_following && conditions.min_following > 0 {
        let idx = binds.len() + 1;
        clauses.push(format!("c.following_count >= ${idx}"));
        binds.push(ConditionBind::Int(conditions.min_following));
    }

    if conditions.require_likes && conditions.min_likes > 0 {
        let idx = binds.len() + 1;
        clauses.push(format!("c.likes_count >= ${idx}"));
        binds.push(ConditionBind::Int(conditions.min_likes));
    }

    if conditions.require_videos && conditions.min_videos > 0 {
        let idx = binds.len() + 1;
        clauses.push(format!("c.video_count >= ${idx}"));
        binds.push(ConditionBind::Int(conditions.min_videos));
    }

    if conditions.require_bio {
        clauses.push("c.has_bio = TRUE".to_string());
        if conditions.min_bio_length > 1 {
            let idx = binds.len() + 1;
            clauses.push(format!("c.bio_length >= ${idx}"));
            binds.push(ConditionBind::SmallInt(conditions.min_bio_length));
        }
    }

    if clauses.is_empty() {
        ("TRUE".to_string(), vec![])
    } else {
        (clauses.join(" AND "), binds)
    }
}

/// Re-evaluate ALL members of the guild for a specific role link (after a config change).
/// Uses SQL-side filtering on denormalized columns. Atomic PUT against RoleLogic.
pub async fn sync_for_role_link(
    guild_id: &str,
    role_id: &str,
    state: &AppState,
) -> Result<(), AppError> {
    let pool = &state.pool;
    let rl_client = &state.rl_client;

    let link = sqlx::query_as::<_, (String, sqlx::types::Json<TikTokConditions>)>(
        "SELECT api_token, conditions FROM role_links WHERE guild_id = $1 AND role_id = $2",
    )
    .bind(guild_id)
    .bind(role_id)
    .fetch_optional(pool)
    .await?;

    let Some((api_token, conditions)) = link else {
        return Ok(());
    };

    let member_ids = auth_gateway::fetch_guild_member_ids(
        &state.http,
        &state.config.auth_gateway_url,
        &state.config.internal_api_key,
        guild_id,
    )
    .await?;

    if member_ids.is_empty() {
        match rl_client
            .upload_users(guild_id, role_id, &[], &api_token)
            .await
        {
            Ok(_) => {}
            Err(AppError::RoleLinkNotFound) => {
                delete_orphan_role_link(guild_id, role_id, pool).await;
                return Ok(());
            }
            Err(e) => return Err(e),
        }
        sqlx::query("DELETE FROM role_assignments WHERE guild_id = $1 AND role_id = $2")
            .bind(guild_id)
            .bind(role_id)
            .execute(pool)
            .await?;
        return Ok(());
    }

    let (_user_count, user_limit) = match rl_client
        .get_user_info(guild_id, role_id, &api_token)
        .await
    {
        Ok(v) => v,
        Err(AppError::RoleLinkNotFound) => {
            delete_orphan_role_link(guild_id, role_id, pool).await;
            return Ok(());
        }
        Err(_) => (0, 100),
    };

    let (where_clause, binds) = build_condition_where(&conditions);

    // Dynamic bind indexes: binds... + members + limit
    let members_bind_idx = binds.len() + 1;
    let limit_bind_idx = binds.len() + 2;

    let query_str = format!(
        "SELECT la.discord_id \
         FROM linked_accounts la \
         JOIN tiktok_stats_cache c ON c.discord_id = la.discord_id \
         WHERE la.discord_id = ANY(${members_bind_idx}::text[]) \
           AND la.revoked_at IS NULL \
           AND ({where_clause}) \
         ORDER BY la.linked_at ASC \
         LIMIT ${limit_bind_idx}",
    );

    let qualifying_ids = exec_condition_query(&query_str, &binds, &member_ids, user_limit, pool).await?;

    if !qualifying_ids.is_empty() && qualifying_ids.len() == user_limit {
        let count_query = format!(
            "SELECT COUNT(*) FROM linked_accounts la \
             JOIN tiktok_stats_cache c ON c.discord_id = la.discord_id \
             WHERE la.discord_id = ANY(${members_bind_idx}::text[]) \
               AND la.revoked_at IS NULL \
               AND ({where_clause})",
        );
        let total: i64 = exec_condition_count(&count_query, &binds, &member_ids, pool)
            .await
            .unwrap_or(qualifying_ids.len() as i64);
        if total as usize > user_limit {
            tracing::warn!(
                guild_id,
                role_id,
                total,
                user_limit,
                "Role link user limit reached: {total} users qualify but limit is {user_limit}"
            );
        }
    }

    match rl_client
        .upload_users(guild_id, role_id, &qualifying_ids, &api_token)
        .await
    {
        Ok(_) => {}
        Err(AppError::RoleLinkNotFound) => {
            delete_orphan_role_link(guild_id, role_id, pool).await;
            return Ok(());
        }
        Err(e) => return Err(e),
    }

    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM role_assignments WHERE guild_id = $1 AND role_id = $2")
        .bind(guild_id)
        .bind(role_id)
        .execute(&mut *tx)
        .await?;
    if !qualifying_ids.is_empty() {
        sqlx::query(
            "INSERT INTO role_assignments (guild_id, role_id, discord_id) \
             SELECT $1, $2, UNNEST($3::text[])",
        )
        .bind(guild_id)
        .bind(role_id)
        .bind(&qualifying_ids)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn exec_condition_query(
    query: &str,
    binds: &[ConditionBind],
    member_ids: &[String],
    limit: usize,
    pool: &PgPool,
) -> Result<Vec<String>, AppError> {
    let mut q = sqlx::query_scalar::<_, String>(query);
    for bind in binds {
        q = match bind {
            ConditionBind::Int(v) => q.bind(*v),
            ConditionBind::SmallInt(v) => q.bind(*v),
        };
    }
    q = q.bind(member_ids);
    q = q.bind(limit as i64);
    Ok(q.fetch_all(pool).await?)
}

async fn exec_condition_count(
    query: &str,
    binds: &[ConditionBind],
    member_ids: &[String],
    pool: &PgPool,
) -> Result<i64, AppError> {
    let mut q = sqlx::query_scalar::<_, i64>(query);
    for bind in binds {
        q = match bind {
            ConditionBind::Int(v) => q.bind(*v),
            ConditionBind::SmallInt(v) => q.bind(*v),
        };
    }
    q = q.bind(member_ids);
    Ok(q.fetch_one(pool).await?)
}

/// Remove every role assignment for this user (called after account unlink).
pub async fn remove_all_assignments(discord_id: &str, state: &AppState) -> Result<(), AppError> {
    let pool = &state.pool;
    let rl_client = &state.rl_client;

    let assignments = sqlx::query_as::<_, (String, String, String)>(
        "SELECT ra.guild_id, ra.role_id, rl.api_token \
         FROM role_assignments ra \
         JOIN role_links rl ON rl.guild_id = ra.guild_id AND rl.role_id = ra.role_id \
         WHERE ra.discord_id = $1",
    )
    .bind(discord_id)
    .fetch_all(pool)
    .await?;

    for (guild_id, role_id, api_token) in &assignments {
        match rl_client
            .remove_user(guild_id, role_id, discord_id, api_token)
            .await
        {
            Ok(_) => {}
            Err(AppError::RoleLinkNotFound) => {
                delete_orphan_role_link(guild_id, role_id, pool).await;
            }
            Err(e) => {
                tracing::error!(guild_id, role_id, discord_id, "Failed to remove user during unlink: {e}");
            }
        }
    }

    sqlx::query("DELETE FROM role_assignments WHERE discord_id = $1")
        .bind(discord_id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Delete a role_link the RoleLogic API reports as gone (403 Invalid or
/// revoked token). CASCADE clears role_assignments. Best-effort: logs DB
/// failures, never propagates them — sync workers must not stop syncing
/// other links over a cleanup hiccup.
async fn delete_orphan_role_link(guild_id: &str, role_id: &str, pool: &PgPool) {
    tracing::warn!(
        guild_id,
        role_id,
        "Role link not found on RoleLogic; removing orphaned local row"
    );
    if let Err(e) = sqlx::query("DELETE FROM role_links WHERE guild_id = $1 AND role_id = $2")
        .bind(guild_id)
        .bind(role_id)
        .execute(pool)
        .await
    {
        tracing::error!(guild_id, role_id, "Failed to delete orphan role_link: {e}");
    }
}
