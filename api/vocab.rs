use http_body_util::BodyExt;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, info, warn};
use vercel_runtime::{AppState, Error, Request, Response, ResponseBody, service_fn};

use std::collections::{HashMap, HashSet};
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

#[derive(Deserialize)]
struct UpdateVocabRequest {
    original_characters: String,
    pinyin: String,
    english: String,
    characters: String,
}

#[derive(Deserialize)]
struct DeleteVocabRequest {
    characters: String,
}

#[derive(Deserialize)]
struct PerfEntry {
    #[allow(dead_code)]
    date: String,
    vocab: String,
    correct: bool,
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
        // ── GET /api/vocab — download all vocab as JSON (with flags if name provided) ────────────────────
        "GET" => {
            let query = req.uri().query().unwrap_or("");
            let mut name = "";
            for pair in query.split('&') {
                let mut parts = pair.splitn(2, '=');
                if let (Some("name"), Some(v)) = (parts.next(), parts.next()) {
                    name = v;
                }
            }

            let raw_items: Vec<String> = con
                .lrange("vocab", 0, -1)
                .await
                .map_err(|e| format!("Redis LRANGE error: {e}"))?;

            if raw_items.is_empty() {
                return error_response(404, "No vocabulary loaded in Redis");
            }

            let vocab_items: Vec<VocabItem> = raw_items
                .iter()
                .filter_map(|s| serde_json::from_str(s).ok())
                .collect();

            if name.is_empty() {
                // Return basic JSON without flags
                let response_data = json!({ "items": vocab_items });
                Ok(Response::builder()
                    .status(200)
                    .header("Content-Type", "application/json")
                    .body(response_data.to_string().into())
                    .unwrap())
            } else {
                // Compute flags based on user performance
                let user_name = name.to_lowercase();
                
                // We need to check performance across all modes to see if it's "never seen"
                // and potentially "weak spot" in any mode. 
                // According to quiz.rs, it checks perf:<name>:<mode>. 
                // For a general "browse" view, we might want to aggregate.
                
                let mut seen_set = HashSet::new();
                let mut stats: HashMap<String, (u32, u32)> = HashMap::new();
                
                let valid_modes = ["zh_to_en", "pinyin_to_en", "en_to_pinyin", "en_to_zh"];
                for mode in valid_modes {
                    let perf_key = format!("perf:{}:{}", user_name, mode);
                    let raw_perf: Vec<String> = con.lrange(&perf_key, 0, -1).await.unwrap_or_default();
                    for raw in raw_perf {
                        if let Ok(entry) = serde_json::from_str::<PerfEntry>(&raw) {
                            seen_set.insert(entry.vocab.clone());
                            let s = stats.entry(entry.vocab).or_insert((0, 0));
                            s.1 += 1;
                            if !entry.correct {
                                s.0 += 1;
                            }
                        }
                    }
                }

                let items_with_flags: Vec<serde_json::Value> = vocab_items
                    .iter()
                    .map(|v| {
                        // The vocab entry in Redis is { pinyin, english, characters }
                        // The PerfEntry.vocab is the question text.
                        // We need to determine if this item was the question.
                        // This is tricky because the question text depends on the mode.
                        // But PerfEntry.vocab is what was recorded.
                        
                        // To be accurate, we should check if any of the possible question 
                        // representations for this item are in seen_set.
                        let representations = [v.characters.as_str(), v.pinyin.as_str(), v.english.as_str()];
                        let is_seen = representations.iter().any(|r| seen_set.contains(*r));
                        let is_fresh = !is_seen;
                        
                        let mut is_weak = false;
                        for r in representations {
                            if let Some(&(wrong, total)) = stats.get(r) {
                                if total >= 1 && (wrong as f64 / total as f64) >= 0.25 {
                                    is_weak = true;
                                    break;
                                }
                            }
                        }

                        json!({
                            "pinyin": v.pinyin,
                            "english": v.english,
                            "characters": v.characters,
                            "is_weak": is_weak,
                            "is_fresh": is_fresh,
                        })
                    })
                    .collect();

                info!(name = %name, count = items_with_flags.len(), "Fetched vocab with flags");

                Ok(Response::builder()
                    .status(200)
                    .header("Content-Type", "application/json")
                    .body(json!({ "items": items_with_flags }).to_string().into())
                    .unwrap())
            }
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

        // ── PUT /api/vocab — update an existing vocab item ───────────────────
        "PUT" => {
            let body_bytes = req
                .into_body()
                .collect()
                .await
                .map_err(|e| format!("Failed to read body: {e}"))?
                .to_bytes();

            let payload: UpdateVocabRequest =
                serde_json::from_slice(&body_bytes).map_err(|e| format!("Invalid JSON body: {e}"))?;

            let original_characters = payload.original_characters.trim().to_string();
            let pinyin     = payload.pinyin.trim().to_string();
            let english    = payload.english.trim().to_string();
            let characters = payload.characters.trim().to_string();

            if original_characters.is_empty() { return error_response(400, "original_characters is required"); }
            if pinyin.is_empty()     { return error_response(400, "pinyin is required"); }
            if english.is_empty()    { return error_response(400, "english is required"); }
            if characters.is_empty() { return error_response(400, "characters is required"); }

            let raw_items: Vec<String> = con
                .lrange("vocab", 0, -1)
                .await
                .map_err(|e| format!("Redis LRANGE error: {e}"))?;

            // Find the index of the item to update
            let mut target_idx: Option<usize> = None;
            for (i, raw) in raw_items.iter().enumerate() {
                if let Ok(item) = serde_json::from_str::<VocabItem>(raw) {
                    if item.characters == original_characters {
                        target_idx = Some(i);
                        break;
                    }
                }
            }

            let idx = match target_idx {
                Some(i) => i,
                None => return error_response(404, "Vocab item not found"),
            };

            // Duplicate check (skip the item being updated)
            for (i, raw) in raw_items.iter().enumerate() {
                if i == idx { continue; }
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

            let updated = VocabItem {
                pinyin: pinyin.clone(),
                english: english.clone(),
                characters: characters.clone(),
            };
            let _: () = con
                .lset("vocab", idx as isize, serde_json::to_string(&updated).unwrap())
                .await
                .map_err(|e| format!("Redis LSET error: {e}"))?;

            info!(original = %original_characters, characters = %characters, "Updated vocab item");

            Ok(Response::builder()
                .status(200)
                .header("Content-Type", "application/json")
                .body(
                    json!({
                        "ok": true,
                        "item": { "pinyin": pinyin, "english": english, "characters": characters },
                    })
                    .to_string()
                    .into(),
                )
                .unwrap())
        }

        // ── DELETE /api/vocab — delete a vocab item ───────────────────────────
        "DELETE" => {
            let body_bytes = req
                .into_body()
                .collect()
                .await
                .map_err(|e| format!("Failed to read body: {e}"))?
                .to_bytes();

            let payload: DeleteVocabRequest =
                serde_json::from_slice(&body_bytes).map_err(|e| format!("Invalid JSON body: {e}"))?;

            let characters = payload.characters.trim().to_string();
            if characters.is_empty() {
                return error_response(400, "characters is required");
            }

            let raw_items: Vec<String> = con
                .lrange("vocab", 0, -1)
                .await
                .map_err(|e| format!("Redis LRANGE error: {e}"))?;

            // Find the exact JSON string to remove
            let mut raw_to_remove: Option<String> = None;
            for raw in &raw_items {
                if let Ok(item) = serde_json::from_str::<VocabItem>(raw) {
                    if item.characters == characters {
                        raw_to_remove = Some(raw.clone());
                        break;
                    }
                }
            }

            let raw = match raw_to_remove {
                Some(r) => r,
                None => return error_response(404, "Vocab item not found"),
            };

            let removed: i64 = con
                .lrem("vocab", 1isize, raw)
                .await
                .map_err(|e| format!("Redis LREM error: {e}"))?;

            if removed == 0 {
                return error_response(404, "Vocab item not found");
            }

            info!(characters = %characters, "Deleted vocab item");

            Ok(Response::builder()
                .status(200)
                .header("Content-Type", "application/json")
                .body(
                    json!({ "ok": true, "total": raw_items.len() - 1 })
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

