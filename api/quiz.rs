use http_body_util::BodyExt;
use rand::seq::SliceRandom;
use rand::Rng;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use tracing::{error, info, warn};
use vercel_runtime::{AppState, Error, Request, Response, ResponseBody, service_fn};

fn error_response(status: u16, msg: &str) -> Result<Response<ResponseBody>, Error> {
    Ok(Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(json!({"error": msg}).to_string().into())
        .unwrap())
}

#[derive(Deserialize)]
struct QuizRequest {
    name: String,
    mode: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct VocabItem {
    pinyin: String,
    english: String,
    characters: String,
}

#[derive(Serialize)]
struct Question {
    question: String,
    options: Vec<String>,
    correct_index: usize,
}

async fn handler(req: Request, _: AppState) -> Result<Response<ResponseBody>, Error> {
    if req.method() != http::Method::POST {
        warn!(method = %req.method(), "Rejected non-POST request");
        return error_response(405, "Method Not Allowed");
    }

    let body_bytes = req
        .into_body()
        .collect()
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to read request body");
            format!("Failed to read body: {e}")
        })?
        .to_bytes();

    let payload: QuizRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        error!(error = %e, "Failed to parse request body");
        format!("Invalid JSON body: {e}")
    })?;

    if payload.name.is_empty() {
        return error_response(400, "name is required");
    }

    let valid_modes = ["zh_to_en", "pinyin_to_en", "en_to_pinyin", "en_to_zh"];
    if !valid_modes.contains(&payload.mode.as_str()) {
        return error_response(400, "invalid mode");
    }

    let redis_url =
        std::env::var("REDIS_URL").map_err(|_| "REDIS_URL environment variable is not set")?;

    let client = redis::Client::open(redis_url.as_str())
        .map_err(|e| format!("Redis client error: {e}"))?;

    let mut con = client
        .get_async_connection()
        .await
        .map_err(|e| format!("Redis connection error: {e}"))?;

    let raw_items: Vec<String> = con
        .lrange("vocab", 0, -1)
        .await
        .map_err(|e| format!("Redis LRANGE error: {e}"))?;

    if raw_items.is_empty() {
        return error_response(500, "Vocabulary not loaded in Redis");
    }

    let mut vocab: Vec<VocabItem> = raw_items
        .iter()
        .filter_map(|s| serde_json::from_str(s).ok())
        .collect();

    if vocab.len() < 10 {
        return error_response(500, "Not enough vocabulary items in Redis");
    }

    let mut rng = rand::thread_rng();
    vocab.shuffle(&mut rng);

    let selected: Vec<VocabItem> = vocab[..10].to_vec();
    let mode = payload.mode.as_str();

    let mut questions: Vec<Question> = Vec::with_capacity(10);

    for item in &selected {
        let (question_text, correct_answer) = match mode {
            "zh_to_en" => (item.characters.clone(), item.english.clone()),
            "pinyin_to_en" => (item.pinyin.clone(), item.english.clone()),
            "en_to_pinyin" => (item.english.clone(), item.pinyin.clone()),
            _ => (item.english.clone(), item.characters.clone()), // en_to_zh
        };

        // Collect all possible answers for this mode from the full vocab pool
        let all_answers: HashSet<String> = vocab
            .iter()
            .map(|v| match mode {
                "zh_to_en" | "pinyin_to_en" => v.english.clone(),
                "en_to_pinyin" => v.pinyin.clone(),
                _ => v.characters.clone(),
            })
            .collect();

        let mut wrong_options: Vec<String> = all_answers
            .into_iter()
            .filter(|a| a != &correct_answer)
            .collect();
        wrong_options.shuffle(&mut rng);
        wrong_options.truncate(4);

        // Safety pad (should never trigger with real vocab)
        while wrong_options.len() < 4 {
            wrong_options.push(format!("—"));
        }

        let correct_pos = rng.gen_range(0..5usize);
        let mut options: Vec<String> = wrong_options.into_iter().take(4).collect();
        options.insert(correct_pos, correct_answer);

        questions.push(Question {
            question: question_text,
            options,
            correct_index: correct_pos,
        });
    }

    info!(name = %payload.name, mode = %payload.mode, count = questions.len(), "Generated quiz");

    Ok(Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(json!({"questions": questions}).to_string().into())
        .unwrap())
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

