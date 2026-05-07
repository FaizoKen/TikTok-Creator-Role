use std::env;

#[derive(Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub session_secret: String,
    pub tiktok_client_key: String,
    pub tiktok_client_secret: String,
    pub base_url: String,
    pub listen_addr: String,
    /// Base URL of the Auth Gateway (no trailing slash, no `/auth` suffix).
    /// Prod: usually the same origin as `BASE_URL` (derived if unset).
    /// Local dev: set to the gateway's local listener, e.g. http://localhost:8090
    pub auth_gateway_url: String,
    /// Shared secret for plugin → gateway /auth/internal/* calls.
    pub internal_api_key: String,
    /// Daily TikTok API quota budget — drives adaptive refresh interval.
    pub tiktok_quota_per_day: i64,
}

/// Extract the origin (scheme://host[:port]) from BASE_URL, dropping any path prefix.
fn derive_origin(base_url: &str) -> String {
    if let Some(scheme_end) = base_url.find("://") {
        let after_scheme = scheme_end + 3;
        if let Some(path_slash) = base_url[after_scheme..].find('/') {
            return base_url[..after_scheme + path_slash].to_string();
        }
    }
    base_url.to_string()
}

impl AppConfig {
    pub fn from_env() -> Self {
        let base_url = env::var("BASE_URL").expect("BASE_URL must be set");
        let auth_gateway_url = env::var("AUTH_GATEWAY_URL")
            .ok()
            .map(|s| s.trim_end_matches('/').to_string())
            .unwrap_or_else(|| derive_origin(&base_url));

        let tiktok_quota_per_day = env::var("TIKTOK_QUOTA_PER_DAY")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(200_000);

        Self {
            database_url: env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
            session_secret: env::var("SESSION_SECRET").expect("SESSION_SECRET must be set"),
            tiktok_client_key: env::var("TIKTOK_CLIENT_KEY")
                .expect("TIKTOK_CLIENT_KEY must be set"),
            tiktok_client_secret: env::var("TIKTOK_CLIENT_SECRET")
                .expect("TIKTOK_CLIENT_SECRET must be set"),
            base_url,
            listen_addr: env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8088".to_string()),
            auth_gateway_url,
            internal_api_key: env::var("INTERNAL_API_KEY")
                .expect("INTERNAL_API_KEY must be set (must match the Auth Gateway's value)"),
            tiktok_quota_per_day,
        }
    }

    pub fn tiktok_oauth_redirect_uri(&self) -> String {
        format!("{}/verify/tiktok/callback", self.base_url)
    }
}
