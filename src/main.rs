use std::sync::Arc;

use axum::routing::{delete, get, post};
use axum::Router;
use sqlx::PgPool;
use tokio::sync::mpsc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

mod config;
mod db;
mod error;
mod models;
mod routes;
mod schema;
mod services;
mod tasks;

use services::rolelogic::RoleLogicClient;
use services::sync::{ConfigSyncEvent, PlayerSyncEvent};
use services::tiktok::TikTokClient;

pub struct AppState {
    pub pool: PgPool,
    pub config: config::AppConfig,
    pub player_sync_tx: mpsc::Sender<PlayerSyncEvent>,
    pub config_sync_tx: mpsc::Sender<ConfigSyncEvent>,
    pub tiktok_client: TikTokClient,
    pub rl_client: RoleLogicClient,
    pub http: reqwest::Client,
    pub verify_html: bytes::Bytes,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tiktok_creator_role=info,tower_http=info".into()),
        )
        .init();

    let app_config = config::AppConfig::from_env();
    let listen_addr = app_config.listen_addr.clone();

    let pool = db::create_pool(&app_config.database_url).await;
    db::run_migrations(&pool).await;
    tracing::info!("Database connected and migrations applied");

    let (player_sync_tx, player_sync_rx) = mpsc::channel::<PlayerSyncEvent>(512);
    let (config_sync_tx, config_sync_rx) = mpsc::channel::<ConfigSyncEvent>(64);

    let tiktok_client = TikTokClient::new(
        &app_config.tiktok_client_key,
        &app_config.tiktok_client_secret,
    );
    let rl_client = RoleLogicClient::new();
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("Failed to build HTTP client");
    let verify_html = bytes::Bytes::from(routes::verification::render_verify_page(
        &app_config.base_url,
    ));

    let state = Arc::new(AppState {
        pool,
        config: app_config,
        player_sync_tx,
        config_sync_tx,
        tiktok_client,
        rl_client,
        http,
        verify_html,
    });

    // Background workers — all spawned, none crash on error (Convention 9).
    tokio::spawn(tasks::refresh_worker::run(Arc::clone(&state)));
    tokio::spawn(tasks::player_sync_worker::run(
        player_sync_rx,
        Arc::clone(&state),
    ));
    tokio::spawn(tasks::config_sync_worker::run(
        config_sync_rx,
        Arc::clone(&state),
    ));
    tokio::spawn(tasks::cleanup_expired(Arc::clone(&state)));

    // All routes are nested under the plugin's path prefix (Convention 23).
    let app = Router::new()
        .nest(
            "/tiktok-creator-role",
            Router::new()
                // RoleLogic plugin contract
                .route("/register", post(routes::plugin::register))
                .route("/config", get(routes::plugin::get_config))
                .route("/config", post(routes::plugin::post_config))
                .route("/config", delete(routes::plugin::delete_config))
                // User-facing verification flow
                .route("/verify", get(routes::verification::verify_page))
                .route("/verify/status", get(routes::verification::status))
                .route("/verify/login", get(routes::verification::login))
                .route("/verify/tiktok", get(routes::verification::tiktok_start))
                .route(
                    "/verify/tiktok/callback",
                    get(routes::verification::tiktok_callback),
                )
                .route("/verify/unlink", post(routes::verification::unlink))
                .route("/verify/logout", post(routes::verification::logout))
                // Health & static
                .route("/favicon.ico", get(routes::health::favicon))
                .route("/health", get(routes::health::health)),
        )
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state);

    tracing::info!("Server starting on {listen_addr}");

    let listener = tokio::net::TcpListener::bind(&listen_addr)
        .await
        .expect("Failed to bind listener");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("Shutdown signal received, draining connections...");
        })
        .await
        .expect("Server error");

    tracing::info!("Server stopped");
}
