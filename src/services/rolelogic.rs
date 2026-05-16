use std::time::Duration;

use crate::error::AppError;

/// Hard cap for a single `PUT /users` request (server-side limit).
const PUT_MAX_USERS: usize = 100_000;
/// Per-chunk cap for the chunked upload flow (server-side limit).
const CHUNK_SIZE: usize = 100_000;
/// Per-request timeout for a single chunk POST. Chunks are short
/// transactions but the payload can be ~2 MB of JSON, so allow headroom.
const CHUNK_TIMEOUT: Duration = Duration::from_secs(120);
/// Per-request timeout for the commit POST. The server may take up to
/// 30 minutes to perform the atomic swap on very large staging sets.
const COMMIT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Body substring RoleLogic returns when our token isn't found server-side.
/// Because `RoleLinkToken` rows cascade on `RoleLink` delete, getting this
/// reliably signals the role link has been deleted upstream.
const RL_LINK_GONE_ERROR_MSG: &str = "Invalid or revoked token";

#[derive(Clone)]
pub struct RoleLogicClient {
    http: reqwest::Client,
    base_url: String,
}

impl RoleLogicClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            http,
            base_url: "https://api-rolelogic.faizo.net".to_string(),
        }
    }

    pub async fn get_user_info(
        &self,
        guild_id: &str,
        role_id: &str,
        token: &str,
    ) -> Result<(usize, usize), AppError> {
        let url = format!(
            "{}/api/role-link/{}/{}/users",
            self.base_url, guild_id, role_id
        );

        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Token {token}"))
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::FORBIDDEN && body.contains(RL_LINK_GONE_ERROR_MSG) {
                return Err(AppError::RoleLinkNotFound);
            }
            return Err(AppError::RoleLogic(format!(
                "Get user info failed: {status} - {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        let user_count = body["data"]["user_count"].as_u64().unwrap_or(0) as usize;
        let user_limit = body["data"]["user_limit"].as_u64().unwrap_or(100) as usize;

        Ok((user_count, user_limit))
    }

    pub async fn add_user(
        &self,
        guild_id: &str,
        role_id: &str,
        user_id: &str,
        token: &str,
    ) -> Result<bool, AppError> {
        let url = format!(
            "{}/api/role-link/{}/{}/users/{}",
            self.base_url, guild_id, role_id, user_id
        );

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Token {token}"))
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();

            if status == reqwest::StatusCode::FORBIDDEN && body.contains(RL_LINK_GONE_ERROR_MSG) {
                return Err(AppError::RoleLinkNotFound);
            }

            if (status == reqwest::StatusCode::BAD_REQUEST || status == reqwest::StatusCode::FORBIDDEN)
                && body.contains("limit")
            {
                let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                let limit = parsed["data"]["user_limit"].as_u64().unwrap_or(100) as usize;
                return Err(AppError::UserLimitReached { limit });
            }

            return Err(AppError::RoleLogic(format!(
                "Add user failed: {status} - {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        Ok(body["data"]["added"].as_bool().unwrap_or(false))
    }

    pub async fn remove_user(
        &self,
        guild_id: &str,
        role_id: &str,
        user_id: &str,
        token: &str,
    ) -> Result<bool, AppError> {
        let url = format!(
            "{}/api/role-link/{}/{}/users/{}",
            self.base_url, guild_id, role_id, user_id
        );

        let resp = self
            .http
            .delete(&url)
            .header("Authorization", format!("Token {token}"))
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::FORBIDDEN && body.contains(RL_LINK_GONE_ERROR_MSG) {
                return Err(AppError::RoleLinkNotFound);
            }
            return Err(AppError::RoleLogic(format!(
                "Remove user failed: {status} - {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        Ok(body["data"]["removed"].as_bool().unwrap_or(false))
    }

    /// Replace the full user set in a single atomic `PUT /users` request.
    /// Server rejects anything over `PUT_MAX_USERS`. For larger sets,
    /// callers should use [`upload_users`] which routes to the chunked flow.
    pub async fn replace_users(
        &self,
        guild_id: &str,
        role_id: &str,
        user_ids: &[String],
        token: &str,
    ) -> Result<usize, AppError> {
        let url = format!(
            "{}/api/role-link/{}/{}/users",
            self.base_url, guild_id, role_id
        );

        let resp = self
            .http
            .put(&url)
            .header("Authorization", format!("Token {token}"))
            .json(user_ids)
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::FORBIDDEN && body.contains(RL_LINK_GONE_ERROR_MSG) {
                return Err(AppError::RoleLinkNotFound);
            }
            return Err(AppError::RoleLogic(format!(
                "Replace users failed: {status} - {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        Ok(body["data"]["user_count"].as_u64().unwrap_or(0) as usize)
    }

    /// High-level user-set upload. Picks the right transport for the size:
    /// - `len <= 100_000`: single atomic `PUT /users`.
    /// - `len > 100_000`: chunked flow (init → chunks → commit).
    pub async fn upload_users(
        &self,
        guild_id: &str,
        role_id: &str,
        user_ids: &[String],
        token: &str,
    ) -> Result<usize, AppError> {
        if user_ids.len() <= PUT_MAX_USERS {
            return self.replace_users(guild_id, role_id, user_ids, token).await;
        }

        let total = user_ids.len();
        tracing::info!(
            guild_id,
            role_id,
            total,
            "Bulk user set exceeds PUT cap; using chunked upload"
        );

        let upload_id = self.start_upload(guild_id, role_id, token).await?;
        let chunk_count = user_ids.chunks(CHUNK_SIZE).count();

        for (i, chunk) in user_ids.chunks(CHUNK_SIZE).enumerate() {
            if let Err(e) = self
                .upload_chunk(guild_id, role_id, &upload_id, chunk, token)
                .await
            {
                tracing::error!(
                    guild_id,
                    role_id,
                    upload_id,
                    chunk_idx = i,
                    chunk_count,
                    "Chunk upload failed; cancelling session: {e}"
                );
                if let Err(cancel_err) = self
                    .cancel_upload(guild_id, role_id, &upload_id, token)
                    .await
                {
                    tracing::warn!(
                        guild_id,
                        role_id,
                        upload_id,
                        "Cancel after chunk failure also failed: {cancel_err}"
                    );
                }
                return Err(e);
            }
        }

        let final_count = self
            .commit_upload(guild_id, role_id, &upload_id, token)
            .await?;
        tracing::info!(
            guild_id,
            role_id,
            upload_id,
            chunks = chunk_count,
            final_count,
            "Chunked upload committed"
        );
        Ok(final_count)
    }

    async fn start_upload(
        &self,
        guild_id: &str,
        role_id: &str,
        token: &str,
    ) -> Result<String, AppError> {
        let url = format!(
            "{}/api/role-link/{}/{}/users/upload",
            self.base_url, guild_id, role_id
        );

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Token {token}"))
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::FORBIDDEN && body.contains(RL_LINK_GONE_ERROR_MSG) {
                return Err(AppError::RoleLinkNotFound);
            }
            return Err(AppError::RoleLogic(format!(
                "Start upload failed: {status} - {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        body["data"]["upload_id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| AppError::RoleLogic("Start upload response missing upload_id".into()))
    }

    async fn upload_chunk(
        &self,
        guild_id: &str,
        role_id: &str,
        upload_id: &str,
        user_ids: &[String],
        token: &str,
    ) -> Result<(), AppError> {
        let url = format!(
            "{}/api/role-link/{}/{}/users/upload/{}/chunk",
            self.base_url, guild_id, role_id, upload_id
        );

        let resp = self
            .http
            .post(&url)
            .timeout(CHUNK_TIMEOUT)
            .header("Authorization", format!("Token {token}"))
            .json(user_ids)
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::RoleLogic(format!(
                "Upload chunk failed: {status} - {body}"
            )));
        }

        Ok(())
    }

    async fn commit_upload(
        &self,
        guild_id: &str,
        role_id: &str,
        upload_id: &str,
        token: &str,
    ) -> Result<usize, AppError> {
        let url = format!(
            "{}/api/role-link/{}/{}/users/upload/{}/commit",
            self.base_url, guild_id, role_id, upload_id
        );

        let resp = self
            .http
            .post(&url)
            .timeout(COMMIT_TIMEOUT)
            .header("Authorization", format!("Token {token}"))
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::RoleLogic(format!(
                "Commit upload failed: {status} - {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        Ok(body["data"]["user_count"].as_u64().unwrap_or(0) as usize)
    }

    async fn cancel_upload(
        &self,
        guild_id: &str,
        role_id: &str,
        upload_id: &str,
        token: &str,
    ) -> Result<(), AppError> {
        let url = format!(
            "{}/api/role-link/{}/{}/users/upload/{}",
            self.base_url, guild_id, role_id, upload_id
        );

        let resp = self
            .http
            .delete(&url)
            .header("Authorization", format!("Token {token}"))
            .send()
            .await
            .map_err(|e| AppError::RoleLogic(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::RoleLogic(format!(
                "Cancel upload failed: {status} - {body}"
            )));
        }

        Ok(())
    }
}
