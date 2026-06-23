use http_body_util::BodyExt;
use rand::seq::SliceRandom;
use rand::Rng;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
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
    /// When true, bias question selection toward vocab the user has struggled with.
    trouble: Option<bool>,
    /// When true, bias question selection toward vocab never seen in this mode.
    fresh: Option<bool>,
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

/// Stored in Redis under `perf:<user>:<mode>` by history.rs.
#[derive(Deserialize)]
struct PerfEntry {
    #[allow(dead_code)]
    date: String,
    vocab: String,
    correct: bool,
}

/// Return the question-side field for a vocab item given the current mode.
fn question_key<'a>(v: &'a VocabItem, mode: &str) -> &'a str {
    match mode {
        "zh_to_en"     => &v.characters,
        "pinyin_to_en" => &v.pinyin,
        _              => &v.english,   // en_to_pinyin, en_to_zh
    }
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

    let vocab: Vec<VocabItem> = raw_items
        .iter()
        .filter_map(|s| serde_json::from_str(s).ok())
        .collect();

    if vocab.len() < 10 {
        return error_response(500, "Not enough vocabulary items in Redis");
    }

    let mode = payload.mode.as_str();

    // ── Fetch perf data up-front (before rng is created — ThreadRng is !Send) ──
    let needs_perf = payload.trouble.unwrap_or(false) || payload.fresh.unwrap_or(false);
    let raw_perf: Vec<String> = if needs_perf {
        let perf_key = format!("perf:{}:{}", payload.name.to_lowercase(), mode);
        con.lrange(&perf_key, 0isize, -1isize).await.unwrap_or_default()
    } else {
        vec![]
    };

    // ── All awaits done — safe to create ThreadRng now ────────────────────
    let mut rng = rand::thread_rng();
    let mut vocab = vocab;
    vocab.shuffle(&mut rng);

    // ── Trouble-vocab / never-seen selection ──────────────────────────────
    let mut trouble_count = 0usize;
    let mut fresh_count   = 0usize;

    // Build the set of vocab strings that have ever been seen, regardless of mode.
    // Used by both trouble and fresh branches.
    let seen_set: HashSet<String> = raw_perf
        .iter()
        .filter_map(|s| serde_json::from_str::<PerfEntry>(s).ok())
        .map(|e| e.vocab)
        .collect();

    let selected: Vec<VocabItem> = if payload.trouble.unwrap_or(false) {
        // Tally wrong / total per vocab item across all time
        let mut stats: HashMap<String, (u32, u32)> = HashMap::new();
        for raw in &raw_perf {
            if let Ok(entry) = serde_json::from_str::<PerfEntry>(raw) {
                let s = stats.entry(entry.vocab).or_insert((0, 0));
                s.1 += 1;
                if !entry.correct {
                    s.0 += 1;
                }
            }
        }

        // Vocab items answered wrong ≥ 25% of the time (minimum 1 attempt)
        let trouble_set: HashSet<String> = stats
            .into_iter()
            .filter(|(_, (wrong, total))| {
                *total >= 1 && (*wrong as f64 / *total as f64) >= 0.25
            })
            .map(|(v, _)| v)
            .collect();

        if trouble_set.is_empty() {
            info!(name = %payload.name, mode = mode, "No trouble vocab found; using random selection");
            vocab[..10].to_vec()
        } else {
            let mut trouble_items: Vec<VocabItem> = vocab
                .iter()
                .filter(|v| trouble_set.contains(question_key(v, mode)))
                .cloned()
                .collect();
            trouble_items.shuffle(&mut rng);
            trouble_items.truncate(10);
            trouble_count = trouble_items.len();

            let trouble_keys: HashSet<String> = trouble_items
                .iter()
                .map(|v| question_key(v, mode).to_string())
                .collect();

            let mut other_items: Vec<VocabItem> = vocab
                .iter()
                .filter(|v| !trouble_keys.contains(question_key(v, mode)))
                .cloned()
                .collect();
            other_items.shuffle(&mut rng);

            trouble_items
                .into_iter()
                .chain(other_items.into_iter())
                .take(10)
                .collect()
        }
    } else if payload.fresh.unwrap_or(false) {
        // Items never seen in this mode come first; fill remainder with random.
        let mut unseen: Vec<VocabItem> = vocab
            .iter()
            .filter(|v| !seen_set.contains(question_key(v, mode)))
            .cloned()
            .collect();
        unseen.shuffle(&mut rng);
        unseen.truncate(10);
        fresh_count = unseen.len();

        if fresh_count == 0 {
            info!(name = %payload.name, mode = mode, "No unseen vocab found; using random selection");
            vocab[..10].to_vec()
        } else {
            let unseen_keys: HashSet<String> = unseen
                .iter()
                .map(|v| question_key(v, mode).to_string())
                .collect();

            let mut other_items: Vec<VocabItem> = vocab
                .iter()
                .filter(|v| !unseen_keys.contains(question_key(v, mode)))
                .cloned()
                .collect();
            other_items.shuffle(&mut rng);

            unseen
                .into_iter()
                .chain(other_items.into_iter())
                .take(10)
                .collect()
        }
    } else {
        vocab[..10].to_vec()
    };

    // ── Build questions (unchanged logic) ─────────────────────────────────
    let mut questions: Vec<Question> = Vec::with_capacity(10);

    for item in &selected {
        let (question_text, correct_answer) = match mode {
            "zh_to_en" => (item.characters.clone(), item.english.clone()),
            "pinyin_to_en" => (item.pinyin.clone(), item.english.clone()),
            "en_to_pinyin" => (item.english.clone(), item.pinyin.clone()),
            _ => (item.english.clone(), item.characters.clone()), // en_to_zh
        };

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

        while wrong_options.len() < 4 {
            wrong_options.push("—".to_string());
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

    info!(
        name = %payload.name,
        mode = mode,
        trouble = payload.trouble.unwrap_or(false),
        trouble_count = trouble_count,
        fresh = payload.fresh.unwrap_or(false),
        fresh_count = fresh_count,
        count = questions.len(),
        "Generated quiz"
    );

    Ok(Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(
            json!({"questions": questions, "trouble_count": trouble_count, "fresh_count": fresh_count})
                .to_string()
                .into(),
        )
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
