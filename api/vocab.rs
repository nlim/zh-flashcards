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

#[derive(Serialize, Deserialize, Clone)]
struct VocabItem {
    pinyin: String,
    english: String,
    characters: String,
}

#[derive(Deserialize)]
struct AddVocabRequest {
    pinyin: String,
    english: String,
    characters: String,
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
        // ── GET /api/vocab — download all vocab as TSV ────────────────────
        "GET" => {
            let raw_items: Vec<String> = con
                .lrange("vocab", 0, -1)
                .await
                .map_err(|e| format!("Redis LRANGE error: {e}"))?;

            if raw_items.is_empty() {
                return error_response(404, "No vocabulary loaded in Redis");
            }

            let vocab: Vec<VocabItem> = raw_items
                .iter()
                .filter_map(|s| serde_json::from_str(s).ok())
                .collect();

            let mut tsv = String::from("Pinyin\tEnglish\tCharacters\n");
            for item in &vocab {
                tsv.push_str(&item.pinyin);
                tsv.push('\t');
                tsv.push_str(&item.english);
                tsv.push('\t');
                tsv.push_str(&item.characters);
                tsv.push('\n');
            }

            info!(count = vocab.len(), "Downloaded vocab as TSV");

            Ok(Response::builder()
                .status(200)
                .header("Content-Type", "text/tab-separated-values; charset=utf-8")
                .header("Content-Disposition", "attachment; filename=\"Chinese Vocab.tsv\"")
                .body(tsv.into())
                .unwrap())
        }

        // ── POST /api/vocab — add a new vocab item ────────────────────────
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

            let payload: AddVocabRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
                error!(error = %e, "Failed to parse request body");
                format!("Invalid JSON body: {e}")
            })?;

            let pinyin     = payload.pinyin.trim().to_string();
            let english    = payload.english.trim().to_string();
            let characters = payload.characters.trim().to_string();

            if pinyin.is_empty() {
                return error_response(400, "pinyin is required");
            }
            if english.is_empty() {
                return error_response(400, "english is required");
            }
            if characters.is_empty() {
                return error_response(400, "characters is required");
            }

            // Check for duplicate (same pinyin, characters, OR english)
            let raw_items: Vec<String> = con
                .lrange("vocab", 0, -1)
                .await
                .map_err(|e| format!("Redis LRANGE error: {e}"))?;

            for raw in &raw_items {
                if let Ok(existing) = serde_json::from_str::<VocabItem>(raw) {
                    if existing.characters == characters {
                        return error_response(409, "duplicate:characters");
                    }
                    if existing.pinyin.to_lowercase() == pinyin.to_lowercase() {
                        return error_response(409, "duplicate:pinyin");
                    }
                    if existing.english.to_lowercase() == english.to_lowercase() {
                        return error_response(409, "duplicate:english");
                    }
                }
            }

            let new_item = VocabItem { pinyin: pinyin.clone(), english: english.clone(), characters: characters.clone() };
            let _: () = con
                .rpush("vocab", serde_json::to_string(&new_item).unwrap())
                .await
                .map_err(|e| format!("Redis RPUSH error: {e}"))?;

            info!(pinyin = %pinyin, english = %english, characters = %characters, "Added new vocab item");

            Ok(Response::builder()
                .status(201)
                .header("Content-Type", "application/json")
                .body(
                    json!({
                        "ok": true,
                        "item": { "pinyin": pinyin, "english": english, "characters": characters },
                        "total": raw_items.len() + 1,
                    })
                    .to_string()
                    .into(),
                )
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

