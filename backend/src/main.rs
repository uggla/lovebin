use std::{net::SocketAddr, path::Path, str::FromStr, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use cookie::{Cookie, SameSite};
use rand::{distr::Alphanumeric, RngExt};
use serde::Serialize;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    pool: SqlitePool,
    config: Arc<Config>,
}

#[derive(Clone)]
struct Config {
    bind_addr: SocketAddr,
    cookie_name: String,
    cookie_secure: bool,
    cookie_same_site: SameSite,
    vote_window: Duration,
    database_url: String,
}

#[derive(Serialize)]
struct HeartsResponse {
    count: i64,
    already_voted: bool,
}

#[derive(Serialize)]
struct VoteResponse {
    count: i64,
    voted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_seconds: Option<u64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cleanup_hearts_backend=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Arc::new(Config::from_env()?);
    ensure_sqlite_parent_exists(&config.database_url)?;

    let pool = connect_database(&config.database_url).await?;

    let bind_addr = config.bind_addr;
    let state = AppState { pool, config };
    let app = build_router(state);

    info!("backend listening on {}", bind_addr);
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .context("failed to bind backend listener")?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("backend server failed")?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn health() -> &'static str {
    "ok"
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/hearts", get(get_hearts).post(post_hearts))
        .route_layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn connect_database(database_url: &str) -> Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(database_url)
        .with_context(|| format!("invalid DATABASE_URL: {database_url}"))?
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .connect_with(options)
        .await
        .context("failed to connect to SQLite")?;

    run_migrations(&pool).await?;
    Ok(pool)
}

async fn run_migrations(pool: &SqlitePool) -> Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("failed to run database migrations")
}

async fn get_hearts(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<HeartsResponse>> {
    let count = current_count(&state.pool).await?;
    let already_voted = match read_voter_cookie(&headers, &state.config.cookie_name) {
        Some(voter_id) => recent_vote(&state.pool, &voter_id, state.config.vote_window)
            .await?
            .is_some(),
        None => false,
    };

    Ok(Json(HeartsResponse {
        count,
        already_voted,
    }))
}

async fn post_hearts(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    let voter_id = read_voter_cookie(&headers, &state.config.cookie_name)
        .unwrap_or_else(generate_voter_id);

    if let Some(retry_after_seconds) =
        recent_vote(&state.pool, &voter_id, state.config.vote_window).await?
    {
        let count = current_count(&state.pool).await?;
        info!("vote refused for voter_id={} retry_after_seconds={}", voter_id, retry_after_seconds);

        let mut response = Json(VoteResponse {
            count,
            voted: false,
            reason: Some("already_voted"),
            retry_after_seconds: Some(retry_after_seconds),
        })
        .into_response();
        add_vote_cookie(response.headers_mut(), &state.config, &voter_id)?;
        return Ok(response);
    }

    let mut tx = state.pool.begin().await?;

    let count = sqlx::query_scalar::<_, i64>(
        "UPDATE hearts SET count = count + 1 WHERE id = 1 RETURNING count",
    )
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO heart_votes (voter_id, last_voted_at)
         VALUES (?1, ?2)
         ON CONFLICT(voter_id) DO UPDATE SET last_voted_at = excluded.last_voted_at",
    )
    .bind(&voter_id)
    .bind(Utc::now().to_rfc3339())
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    info!("vote accepted count={}", count);

    let mut response = Json(VoteResponse {
        count,
        voted: true,
        reason: None,
        retry_after_seconds: None,
    })
    .into_response();
    add_vote_cookie(response.headers_mut(), &state.config, &voter_id)?;
    Ok(response)
}

async fn current_count(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count FROM hearts WHERE id = 1")
        .fetch_one(pool)
        .await
}

async fn recent_vote(
    pool: &SqlitePool,
    voter_id: &str,
    vote_window: Duration,
) -> Result<Option<u64>, sqlx::Error> {
    let last_voted_at = sqlx::query_scalar::<_, String>(
        "SELECT last_voted_at FROM heart_votes WHERE voter_id = ?1",
    )
    .bind(voter_id)
    .fetch_optional(pool)
    .await?;

    let Some(last_voted_at) = last_voted_at else {
        return Ok(None);
    };

    let Some(last_vote) = parse_sqlite_time(&last_voted_at) else {
        warn!("ignoring unreadable vote timestamp for voter_id={}", voter_id);
        return Ok(None);
    };

    let elapsed = Utc::now()
        .signed_duration_since(last_vote)
        .to_std()
        .unwrap_or_default();

    if elapsed >= vote_window {
        Ok(None)
    } else {
        Ok(Some((vote_window - elapsed).as_secs()))
    }
}

fn parse_sqlite_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|dt| Utc.from_utc_datetime(&dt))
        })
}

fn read_voter_cookie(headers: &HeaderMap, cookie_name: &str) -> Option<String> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;

    cookie_header
        .split(';')
        .filter_map(|part| Cookie::parse(part.trim()).ok())
        .find(|cookie| cookie.name() == cookie_name)
        .map(|cookie| cookie.value().to_owned())
        .filter(|value| !value.is_empty())
}

fn add_vote_cookie(headers: &mut HeaderMap, config: &Config, voter_id: &str) -> Result<()> {
    let mut cookie = Cookie::build((config.cookie_name.clone(), voter_id.to_owned()))
        .path("/")
        .http_only(true)
        .secure(config.cookie_secure)
        .same_site(config.cookie_same_site)
        .max_age(cookie::time::Duration::seconds(
            config.vote_window.as_secs().try_into().unwrap_or(i64::MAX),
        ))
        .build();

    cookie.set_same_site(config.cookie_same_site);
    let value = HeaderValue::from_str(&cookie.to_string()).context("invalid Set-Cookie header")?;
    headers.append(header::SET_COOKIE, value);
    Ok(())
}

fn generate_voter_id() -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect()
}

fn ensure_sqlite_parent_exists(database_url: &str) -> Result<()> {
    let Some(path) = database_url.strip_prefix("sqlite:") else {
        return Ok(());
    };

    if path == ":memory:" {
        return Ok(());
    }

    let path = path.trim_start_matches("//");
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create database directory {}", parent.display()))?;
        }
    }

    Ok(())
}

impl Config {
    fn from_env() -> Result<Self> {
        let database_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:data/hearts.db".to_owned());
        let bind_addr = std::env::var("BIND_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:3000".to_owned())
            .parse()
            .context("BIND_ADDR must be a socket address, for example 0.0.0.0:3000")?;
        let cookie_name =
            std::env::var("COOKIE_NAME").unwrap_or_else(|_| "cleanup_heart_vote".to_owned());
        let cookie_secure = parse_bool_env("COOKIE_SECURE", true)?;
        let cookie_same_site = parse_same_site(
            &std::env::var("COOKIE_SAME_SITE").unwrap_or_else(|_| "Lax".to_owned()),
        )?;
        let vote_window_seconds = std::env::var("VOTE_WINDOW_SECONDS")
            .ok()
            .map(|value| value.parse::<u64>())
            .transpose()
            .context("VOTE_WINDOW_SECONDS must be a positive integer")?
            .unwrap_or(172_800);

        Ok(Self {
            bind_addr,
            cookie_name,
            cookie_secure,
            cookie_same_site,
            vote_window: Duration::from_secs(vote_window_seconds),
            database_url,
        })
    }
}

fn parse_bool_env(name: &str, default: bool) -> Result<bool> {
    match std::env::var(name) {
        Ok(value) => match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => anyhow::bail!("{name} must be true or false"),
        },
        Err(_) => Ok(default),
    }
}

fn parse_same_site(value: &str) -> Result<SameSite> {
    match value.to_ascii_lowercase().as_str() {
        "strict" => Ok(SameSite::Strict),
        "lax" => Ok(SameSite::Lax),
        "none" => Ok(SameSite::None),
        _ => anyhow::bail!("COOKIE_SAME_SITE must be Strict, Lax, or None"),
    }
}

struct ApiError(anyhow::Error);

type ApiResult<T> = std::result::Result<T, ApiError>;

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(error: E) -> Self {
        Self(error.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        warn!("api error: {:#}", self.0);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "internal_server_error"
            })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::{
        body::{to_bytes, Body},
        http::{Method, Request},
    };
    use serde_json::Value;
    use tower::ServiceExt;

    async fn test_app(vote_window: Duration) -> (Router, SqlitePool) {
        let options = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        run_migrations(&pool).await.unwrap();

        let config = Arc::new(Config {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            cookie_name: "test_heart_vote".to_owned(),
            cookie_secure: false,
            cookie_same_site: SameSite::Lax,
            vote_window,
            database_url: "sqlite::memory:".to_owned(),
        });

        let app = build_router(AppState {
            pool: pool.clone(),
            config,
        });

        (app, pool)
    }

    async fn request_json(app: Router, request: Request<Body>) -> (HeaderMap, Value) {
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&body).unwrap();

        (headers, json)
    }

    fn request(method: Method, uri: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap()
    }

    fn cookie_pair(headers: &HeaderMap) -> String {
        headers
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned()
    }

    #[tokio::test]
    async fn get_hearts_returns_initial_count_and_vote_state() {
        let (app, _pool) = test_app(Duration::from_secs(60)).await;

        let (_headers, json) = request_json(app, request(Method::GET, "/api/hearts")).await;

        assert_eq!(json["count"], 0);
        assert_eq!(json["already_voted"], false);
    }

    #[tokio::test]
    async fn post_hearts_accepts_first_vote_and_sets_http_only_cookie() {
        let (app, pool) = test_app(Duration::from_secs(60)).await;

        let (headers, json) = request_json(app, request(Method::POST, "/api/hearts")).await;

        assert_eq!(json["count"], 1);
        assert_eq!(json["voted"], true);
        assert!(json.get("reason").is_none());

        let set_cookie = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
        assert!(set_cookie.starts_with("test_heart_vote="));
        assert!(set_cookie.contains("HttpOnly"));
        assert!(set_cookie.contains("SameSite=Lax"));
        assert!(set_cookie.contains("Max-Age=60"));
        assert!(!set_cookie.contains("Secure"));
        assert_eq!(current_count(&pool).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn post_hearts_rejects_second_vote_with_same_cookie() {
        let (app, pool) = test_app(Duration::from_secs(60)).await;

        let (headers, first_json) =
            request_json(app.clone(), request(Method::POST, "/api/hearts")).await;
        let cookie = cookie_pair(&headers);
        let second_request = Request::builder()
            .method(Method::POST)
            .uri("/api/hearts")
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap();

        let (_headers, second_json) = request_json(app, second_request).await;

        assert_eq!(first_json["count"], 1);
        assert_eq!(second_json["count"], 1);
        assert_eq!(second_json["voted"], false);
        assert_eq!(second_json["reason"], "already_voted");
        assert!(second_json["retry_after_seconds"].as_u64().unwrap() <= 60);
        assert_eq!(current_count(&pool).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn get_hearts_reports_already_voted_when_cookie_vote_is_recent() {
        let (app, _pool) = test_app(Duration::from_secs(60)).await;

        let (headers, _json) = request_json(app.clone(), request(Method::POST, "/api/hearts")).await;
        let get_request = Request::builder()
            .method(Method::GET)
            .uri("/api/hearts")
            .header(header::COOKIE, cookie_pair(&headers))
            .body(Body::empty())
            .unwrap();

        let (_headers, json) = request_json(app, get_request).await;

        assert_eq!(json["count"], 1);
        assert_eq!(json["already_voted"], true);
    }

    #[tokio::test]
    async fn post_hearts_allows_vote_after_window_expires() {
        let (app, pool) = test_app(Duration::from_secs(60)).await;

        let (headers, _json) = request_json(app.clone(), request(Method::POST, "/api/hearts")).await;
        let cookie = cookie_pair(&headers);
        let voter_id = cookie.strip_prefix("test_heart_vote=").unwrap();
        let old_vote_time = (Utc::now() - chrono::Duration::seconds(61)).to_rfc3339();

        sqlx::query("UPDATE heart_votes SET last_voted_at = ?1 WHERE voter_id = ?2")
            .bind(old_vote_time)
            .bind(voter_id)
            .execute(&pool)
            .await
            .unwrap();

        let second_request = Request::builder()
            .method(Method::POST)
            .uri("/api/hearts")
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap();
        let (_headers, json) = request_json(app, second_request).await;

        assert_eq!(json["count"], 2);
        assert_eq!(json["voted"], true);
        assert_eq!(current_count(&pool).await.unwrap(), 2);
    }

    #[test]
    fn parses_sqlite_and_rfc3339_timestamps() {
        assert!(parse_sqlite_time("2026-05-25T14:27:09Z").is_some());
        assert!(parse_sqlite_time("2026-05-25 14:27:09").is_some());
        assert!(parse_sqlite_time("pas une date").is_none());
    }
}
