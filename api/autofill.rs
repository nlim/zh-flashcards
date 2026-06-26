use http_body_util::BodyExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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
struct AutofillRequest {
    english: String,
}

#[derive(Serialize, Deserialize)]
struct AutofillResult {
    pinyin: String,
    characters: String,
}

async fn call_gemini(english: &str) -> Result<AutofillResult, String> {
    let api_key = std::env::var("GEMINI_API_KEY")
        .map_err(|_| "GEMINI_API_KEY environment variable is not set".to_string())?;

    let url = "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.1-flash-lite:generateContent";

    info!(english, "Calling Gemini for vocab autofill");

    let prompt = format!(
        "For the Chinese vocabulary word whose English meaning is \"{english}\", \
        provide the standard Mandarin Chinese simplified characters and their pinyin with tone marks. \
        Return only a single most common translation. \
        pinyin must use tone-marked romanisation (e.g. \"nǐ hǎo\"), not tone numbers. \
        characters must be simplified Chinese characters only.",
    );

    let body = json!({
        "contents": [{
            "role": "user",
            "parts": [{ "text": prompt }]
        }],
        "generationConfig": {
            "thinkingConfig": { "thinkingBudget": 0 },
            "responseMimeType": "application/json",
            "responseSchema": {
                "type": "object",
                "required": ["pinyin", "characters"],
                "properties": {
                    "pinyin": {
                        "type": "string",
                        "description": "Mandarin pinyin with tone marks, e.g. \"nǐ hǎo\""
                    },
                    "characters": {
                        "type": "string",
                        "description": "Simplified Chinese characters, e.g. \"你好\""
                    }
                }
            }
        }
    });

    let client = reqwest::Client::new();
    let res = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("x-goog-api-key", &api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            error!(error = %e, "Gemini HTTP request failed");
            format!("Gemini request failed: {e}")
        })?;

    let status = res.status();
    info!(status = %status, "Received response from Gemini");

    let response_json: Value = res.json().await.map_err(|e| {
        error!(error = %e, "Failed to parse Gemini response as JSON");
        format!("Failed to parse Gemini response: {e}")
    })?;

    if !status.is_success() {
        let msg = response_json["error"]["message"]
            .as_str()
            .unwrap_or("Unknown Gemini API error");
        error!(status = %status, gemini_error = msg, "Gemini API returned an error");
        return Err(format!("Gemini API error {status}: {msg}"));
    }

    let text = response_json["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .ok_or_else(|| {
            error!(response = %response_json, "Could not extract text from Gemini response");
            "Could not extract text from Gemini response".to_string()
        })?;

    info!(raw_text = text, "Raw JSON from Gemini");

    serde_json::from_str::<AutofillResult>(text.trim()).map_err(|e| {
        error!(error = %e, raw_text = text, "Failed to deserialize Gemini autofill JSON");
        format!("Failed to parse autofill JSON: {e}\nRaw: {text}")
    })
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

    let payload: AutofillRequest = serde_json::from_slice(&body_bytes).map_err(|e| {
        error!(error = %e, "Failed to parse request body");
        format!("Invalid JSON body: {e}")
    })?;

    let english = payload.english.trim().to_string();
    if english.is_empty() {
        return error_response(400, "english is required");
    }

    match call_gemini(&english).await {
        Ok(result) => {
            info!(
                english = %english,
                pinyin = %result.pinyin,
                characters = %result.characters,
                "Autofill successful"
            );
            Ok(Response::builder()
                .status(200)
                .header("Content-Type", "application/json")
                .body(
                    json!({
                        "pinyin":     result.pinyin,
                        "characters": result.characters,
                    })
                    .to_string()
                    .into(),
                )
                .unwrap())
        }
        Err(e) => {
            error!(error = %e, "Autofill Gemini call failed");
            error_response(500, &e)
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

