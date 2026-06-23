use http_body_util::BodyExt;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, info, warn};
use vercel_runtime::{AppState, Error, Request, Response, ResponseBody, service_fn};

fn error_response(status: u16, msg: &str) -> Result<Response<ResponseBody>, Error> {
    Ok(Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(json!({"error": msg}).to_string().into())
        .unwrap())
}

/// One answer recorded per quiz question, sent by the frontend when saving a session.
#[derive(Deserialize)]
struct AnswerRecord {
    vocab: String,   // the question text shown to the user (chars / pinyin / english)
    correct: bool,
}

/// Stored in Redis under `perf:<user>:<mode>` for per-vocab tracking.
#[derive(Serialize, Deserialize)]
struct PerfEntry {
    date: String,
    vocab: String,
    correct: bool,
}

#[derive(Deserialize)]
struct SaveRequest {
    name: String,
    correct: u32,
    total: u32,
    mode: String,
    date: String,
    /// Per-question answer records; optional for backwards compatibility.
    answers: Option<Vec<AnswerRecord>>,
}

#[derive(Serialize, Deserialize)]
struct Session {
    date: String,
    correct: u32,
    total: u32,
    mode: String,
}

async fn handler(req: Request, _: AppState) -> Result<Response<ResponseBody>, Error> {
    let redis_url =
        std::env::var("REDIS_URL").map_err(|_| "REDIS_URL environment variable is not set")?;

    let client = redis::Client::open(redis_url.as_str())
        .map_err(|e| format!("Redis client error: {e}"))?;

    let mut con = client
        .get_async_connection()
        .await
        .map_err(|e| format!("Redis connection error: {e}"))?;

    match req.method().as_str() {
        "GET" => {
            // Parse query params: name, page
            let query = req.uri().query().unwrap_or("");
            let mut name = "";
            let mut page: i64 = 1;
            for pair in query.split('&') {
                let mut parts = pair.splitn(2, '=');
                match (parts.next(), parts.next()) {
                    (Some("name"), Some(v)) => name = v,
                    (Some("page"), Some(v)) => page = v.parse().unwrap_or(1).max(1),
                    _ => {}
                }
            }

            if name.is_empty() {
                return error_response(400, "name query parameter is required");
            }

            const PAGE_SIZE: i64 = 10;

            let redis_key = format!("sessions:{}", name.to_lowercase());

            let total_count: i64 = con
                .llen(&redis_key)
                .await
                .map_err(|e| format!("Redis LLEN error: {e}"))?;

            let total_pages = ((total_count + PAGE_SIZE - 1) / PAGE_SIZE).max(1);
            let page = page.min(total_pages);

            // Newest items are at the end of the list (rpush).
            // For page 1 we want the last PAGE_SIZE items, for page 2 the prior PAGE_SIZE, etc.
            let end: i64 = total_count - (page - 1) * PAGE_SIZE - 1;
            let start: i64 = (end - PAGE_SIZE + 1).max(0);

            let raw_sessions: Vec<String> = con
                .lrange(&redis_key, start as isize, end as isize)
                .await
                .map_err(|e| format!("Redis LRANGE error: {e}"))?;

            let mut sessions: Vec<Session> = raw_sessions
                .iter()
                .filter_map(|s| serde_json::from_str(s).ok())
                .collect();

            sessions.reverse(); // most recent first within the page

            info!(
                name = name,
                page = page,
                total_pages = total_pages,
                count = sessions.len(),
                "Fetched sessions"
            );

            Ok(Response::builder()
                .status(200)
                .header("Content-Type", "application/json")
                .body(
                    json!({
                        "sessions": sessions,
                        "page": page,
                        "total_pages": total_pages,
                        "total_count": total_count,
                    })
                    .to_string()
                    .into(),
                )
                .unwrap())
        }

        "POST" => {
            let body_bytes = req
                .into_body()
                .collect()
                .await
                .map_err(|e| {
                    error!(error = %e, "Failed to read request body");
                    format!("Failed to read body: {e}")
                })?
                .to_bytes();

            let payload: SaveRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
                error!(error = %e, "Failed to parse request body");
                format!("Invalid JSON body: {e}")
            })?;

            if payload.name.is_empty() {
                return error_response(400, "name is required");
            }

            let session = Session {
                date: payload.date.clone(),
                correct: payload.correct,
                total: payload.total,
                mode: payload.mode.clone(),
            };

            let redis_key = format!("sessions:{}", payload.name.to_lowercase());
            let _: () = con
                .rpush(&redis_key, serde_json::to_string(&session).unwrap())
                .await
                .map_err(|e| format!("Redis RPUSH error: {e}"))?;

            // Persist per-answer performance data if provided
            if let Some(answers) = &payload.answers {
                if !answers.is_empty() {
                    let perf_key = format!("perf:{}:{}", payload.name.to_lowercase(), payload.mode);
                    let mut pipe = redis::pipe();
                    for answer in answers {
                        let entry = serde_json::to_string(&PerfEntry {
                            date: payload.date.clone(),
                            vocab: answer.vocab.clone(),
                            correct: answer.correct,
                        })
                        .unwrap();
                        pipe.rpush(&perf_key, entry).ignore();
                    }
                    let (): () = pipe.query_async(&mut con)
                        .await
                        .map_err(|e| format!("Redis pipeline (perf) error: {e}"))?;

                    info!(
                        name = %payload.name,
                        mode = %payload.mode,
                        count = answers.len(),
                        "Saved per-vocab performance entries"
                    );
                }
            }

            info!(
                name = %payload.name,
                correct = payload.correct,
                total = payload.total,
                mode = %payload.mode,
                "Saved session"
            );

            Ok(Response::builder()
                .status(200)
                .header("Content-Type", "application/json")
                .body(json!({"ok": true}).to_string().into())
                .unwrap())
        }

        _ => {
            warn!(method = %req.method(), "Rejected unsupported method");
            error_response(405, "Method Not Allowed")
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_ansi(false)
        .with_target(false)
        .without_time()
        .init();

    let app = service_fn(handler);
    vercel_runtime::run(app).await
}
