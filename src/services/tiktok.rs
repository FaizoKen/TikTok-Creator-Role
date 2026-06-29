//! TikTok Login Kit (OAuth 2.0) + Display API client.
//!
//! - Authorize URL: `https://www.tiktok.com/v2/auth/authorize/`
//! - Token endpoint: `POST https://open.tiktokapis.com/v2/oauth/token/`
//! - Display API:    `GET  https://open.tiktokapis.com/v2/user/info/`
//! - Revoke:         `POST https://open.tiktokapis.com/v2/oauth/revoke/`
//!
//! Scopes used: `user.info.basic`, `user.info.profile`, `user.info.stats`.

use governor::{Quota, RateLimiter};
use std::num::NonZeroU32;
use std::sync::Arc;

use crate::error::AppError;

const AUTHORIZE_URL: &str = "https://www.tiktok.com/v2/auth/authorize/";
const TOKEN_URL: &str = "https://open.tiktokapis.com/v2/oauth/token/";
const REVOKE_URL: &str = "https://open.tiktokapis.com/v2/oauth/revoke/";
const USER_INFO_URL: &str = "https://open.tiktokapis.com/v2/user/info/";

pub const SCOPES: &str = "user.info.basic,user.info.profile,user.info.stats";

const USER_INFO_FIELDS: &str = "open_id,union_id,username,display_name,bio_description,is_verified,follower_count,following_count,likes_count,video_count,avatar_url";

#[derive(Debug, Clone)]
pub struct TikTokTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub refresh_expires_in: i64,
    pub open_id: String,
    pub scope: String,
}

#[derive(Debug, Clone)]
pub struct TikTokUserInfo {
    pub open_id: String,
    pub union_id: Option<String>,
    pub username: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub bio_description: String,
    pub is_verified: bool,
    pub follower_count: i64,
    pub following_count: i64,
    pub likes_count: i64,
    pub video_count: i64,
}

#[derive(Debug, serde::Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    refresh_expires_in: Option<i64>,
    open_id: Option<String>,
    scope: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct UserInfoEnvelope {
    data: Option<UserInfoData>,
    error: Option<TikTokErrorObj>,
}

#[derive(Debug, serde::Deserialize)]
struct UserInfoData {
    user: Option<UserInfoUser>,
}

#[derive(Debug, serde::Deserialize, Default)]
struct UserInfoUser {
    #[serde(default)]
    open_id: String,
    #[serde(default)]
    union_id: Option<String>,
    #[serde(default)]
    username: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    avatar_url: Option<String>,
    #[serde(default)]
    bio_description: String,
    #[serde(default)]
    is_verified: bool,
    #[serde(default)]
    follower_count: i64,
    #[serde(default)]
    following_count: i64,
    #[serde(default)]
    likes_count: i64,
    #[serde(default)]
    video_count: i64,
}

#[derive(Debug, serde::Deserialize, Default)]
struct TikTokErrorObj {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    log_id: String,
}

pub struct TikTokClient {
    http: reqwest::Client,
    pub client_key: String,
    client_secret: String,
    rate_limiter: Arc<
        RateLimiter<
            governor::state::NotKeyed,
            governor::state::InMemoryState,
            governor::clock::DefaultClock,
        >,
    >,
}

impl TikTokClient {
    pub fn new(client_key: &str, client_secret: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("Failed to build HTTP client");

        // ~5 requests per second is the documented per-app cap on Display API.
        let quota = Quota::per_second(NonZeroU32::new(5).unwrap());
        let rate_limiter = Arc::new(RateLimiter::direct(quota));

        Self {
            http,
            client_key: client_key.to_string(),
            client_secret: client_secret.to_string(),
            rate_limiter,
        }
    }

    pub async fn wait_for_permit(&self) {
        self.rate_limiter.until_ready().await;
    }

    /// Build the authorize URL. The caller is responsible for storing `state`
    /// in `oauth_states` for CSRF verification.
    pub fn build_authorize_url(&self, redirect_uri: &str, state: &str) -> String {
        let mut url = url_form(&[
            ("client_key", &self.client_key),
            ("response_type", "code"),
            ("scope", SCOPES),
            ("redirect_uri", redirect_uri),
            ("state", state),
        ]);
        url.insert(0, '?');
        format!("{AUTHORIZE_URL}{url}")
    }

    /// Exchange an OAuth authorization code for tokens.
    pub async fn exchange_code(
        &self,
        code: &str,
        redirect_uri: &str,
    ) -> Result<TikTokTokens, AppError> {
        self.wait_for_permit().await;
        let resp = self
            .http
            .post(TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Cache-Control", "no-cache")
            .form(&[
                ("client_key", self.client_key.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("code", code),
                ("grant_type", "authorization_code"),
                ("redirect_uri", redirect_uri),
            ])
            .send()
            .await
            .map_err(|e| AppError::TikTokApi(format!("Token exchange request failed: {e}")))?;

        parse_token_response(resp).await
    }

    /// Refresh an expired access token. TikTok rotates the refresh token on every refresh.
    pub async fn refresh_token(&self, refresh_token: &str) -> Result<TikTokTokens, AppError> {
        self.wait_for_permit().await;
        let resp = self
            .http
            .post(TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Cache-Control", "no-cache")
            .form(&[
                ("client_key", self.client_key.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ])
            .send()
            .await
            .map_err(|e| AppError::TikTokApi(format!("Token refresh request failed: {e}")))?;

        parse_token_response(resp).await
    }

    /// Best-effort revoke. Errors are logged but not propagated; caller should
    /// proceed with local cleanup regardless.
    pub async fn revoke_token(&self, access_token: &str) {
        let _ = self.wait_for_permit().await;
        let result = self
            .http
            .post(REVOKE_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[
                ("client_key", self.client_key.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("token", access_token),
            ])
            .send()
            .await;
        if let Err(e) = result {
            tracing::warn!("TikTok revoke failed (best-effort): {e}");
        }
    }

    /// Fetch the authenticated user's info via the Display API.
    pub async fn fetch_user_info(&self, access_token: &str) -> Result<TikTokUserInfo, AppError> {
        self.wait_for_permit().await;
        let resp = self
            .http
            .get(USER_INFO_URL)
            .query(&[("fields", USER_INFO_FIELDS)])
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| AppError::TikTokApi(format!("user_info request failed: {e}")))?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(AppError::TikTokTokenExpired);
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(AppError::TikTokRateLimited);
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::TikTokApi(format!(
                "user_info returned {status}: {body}"
            )));
        }

        let body: UserInfoEnvelope = resp
            .json()
            .await
            .map_err(|e| AppError::TikTokApi(format!("user_info response not JSON: {e}")))?;

        if let Some(err) = body.error.as_ref() {
            // The Display API always returns an `error` object — `code: "ok"` means success.
            if !err.code.is_empty() && err.code != "ok" {
                return Err(map_display_error(err));
            }
        }

        let user = body
            .data
            .and_then(|d| d.user)
            .ok_or_else(|| AppError::TikTokApi("user_info missing data.user".into()))?;

        Ok(TikTokUserInfo {
            open_id: user.open_id,
            union_id: user.union_id,
            username: user.username,
            display_name: user.display_name,
            avatar_url: user.avatar_url,
            bio_description: user.bio_description,
            is_verified: user.is_verified,
            follower_count: user.follower_count,
            following_count: user.following_count,
            likes_count: user.likes_count,
            video_count: user.video_count,
        })
    }
}

fn map_display_error(err: &TikTokErrorObj) -> AppError {
    match err.code.as_str() {
        "access_token_invalid" | "scope_not_authorized" => AppError::TikTokTokenExpired,
        "rate_limit_exceeded" => AppError::TikTokRateLimited,
        _ => AppError::TikTokApi(format!(
            "Display API error code={} message={} log_id={}",
            err.code, err.message, err.log_id
        )),
    }
}

async fn parse_token_response(resp: reqwest::Response) -> Result<TikTokTokens, AppError> {
    let status = resp.status();
    let body: TokenResponse = resp
        .json()
        .await
        .map_err(|e| AppError::TikTokApi(format!("Token response not JSON: {e}")))?;

    if let Some(err) = body.error.as_ref() {
        if !err.is_empty() {
            let desc = body.error_description.unwrap_or_default();
            // `invalid_grant` typically indicates the refresh token has been revoked
            // by the user (or simply expired past the 365-day refresh TTL).
            if err == "invalid_grant" {
                return Err(AppError::TikTokTokenRevoked);
            }
            return Err(AppError::TikTokApi(format!(
                "OAuth error {status}: {err} - {desc}"
            )));
        }
    }

    let access_token = body
        .access_token
        .ok_or_else(|| AppError::TikTokApi("Missing access_token".into()))?;
    let refresh_token = body
        .refresh_token
        .ok_or_else(|| AppError::TikTokApi("Missing refresh_token".into()))?;
    let open_id = body.open_id.unwrap_or_default();
    let expires_in = body.expires_in.unwrap_or(86_400); // default ~24h
    let refresh_expires_in = body.refresh_expires_in.unwrap_or(31_536_000); // default ~365d
    let scope = body.scope.unwrap_or_default();

    Ok(TikTokTokens {
        access_token,
        refresh_token,
        expires_in,
        refresh_expires_in,
        open_id,
        scope,
    })
}

/// Tiny x-www-form-urlencoded encoder for stable URL building (no extra deps).
fn url_form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}
