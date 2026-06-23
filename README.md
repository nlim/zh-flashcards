# 🏮 中文 Flashcards

A browser-based Chinese vocabulary quiz app. Vocabulary is stored in Redis, quiz logic runs as Rust serverless functions on Vercel, and session history is persisted per-user in Redis.

---

## User Experience

### 1. Welcome screen
Enter your name. This is used as the key for storing and retrieving your personal quiz history — no account or password required.

### 2. Mode selection
Choose one of four quiz directions:

| Mode | Prompt | Answer type |
|---|---|---|
| **Characters → English** | See a Chinese character | Pick the English meaning |
| **Pinyin → English** | See a pinyin word | Pick the English meaning |
| **English → Pinyin** | See an English word | Pick the correct pinyin |
| **English → Characters** | See an English word | Pick the correct Chinese character |

### 3. Quiz
Ten questions are presented one at a time, each with five multiple-choice options (A–E). After selecting an answer:
- Correct answers are highlighted green.
- Wrong answers are highlighted red, and the correct answer is revealed.
- Click **Next Question** (or **See Results** on the last question).

### 4. Results screen
After all ten questions a score circle shows your result (`correct / 10`, percentage). A full question-by-question breakdown lists what you got right and wrong. Results are automatically saved to your history.

### 5. History screen
Accessible via the **📊 History** button in the navbar (visible after entering your name) or from the results screen. Shows your past sessions in reverse-chronological order, paginated at **10 sessions per page**, with Previous / Next controls.

### 6. Progress screen
Accessible via the **📈 Progress** button in the navbar or from the results screen. Plots your quiz scores over time so you can see how each mode is trending.

- **Preset date ranges** — `1M | 3M | 6M | 1Y` buttons instantly reload the chart for that window. Default on open is the last 1 month.
- **Custom range** — select the Custom button to reveal From / To date pickers and an Apply button.
- **Chart** — a scatter + line chart with one colour-coded series per mode:
  - 🔴 Characters → English
  - 🔵 Pinyin → English
  - 🟢 English → Pinyin
  - 🟠 English → Characters
- X-axis shows dates (unit auto-adjusts to day / week / month based on range width); Y-axis is 0–100%.
- Each point is one quiz session; hovering shows the exact timestamp and score.
- Modes with no sessions in the selected range are hidden from the chart and legend automatically.

---

## Architecture

```
Browser (index.html)
       │  static file served by Vercel CDN
       │
       ├──POST /api/quiz    ──▶  Rust serverless function  (api/quiz.rs)
       │                              │
       └──GET/POST /api/history ──▶  Rust serverless function  (api/history.rs)
                                          │
                                     Redis (Upstash or any Redis-compatible store)
                                      ├── "vocab"            (List of vocab JSON)
                                      └── "sessions:<name>"  (List of session JSON)
```

The Progress screen is purely client-side — it calls `/api/history` (fetching all pages in parallel) and renders the data with **Chart.js 4** using the `chartjs-adapter-date-fns` time scale adapter, both loaded from jsDelivr CDN.

All requests pass through a Vercel Edge Middleware (`middleware.ts`) that enforces HTTP Basic Auth before anything reaches the static file or the Rust functions.

---

## Technical Details

### Deployment platform — Vercel

`vercel.json` configures a single rewrite so that `/` resolves to `index.html`:

```json
{
  "rewrites": [
    { "source": "/", "destination": "/index.html" }
  ]
}
```

Vercel automatically detects `api/*.rs` binaries built by Cargo and deploys them as serverless functions. The release profile uses `lto = "fat"` and `codegen-units = 1` for the smallest, fastest cold-start binary.

### Serverless functions — Rust

Each API endpoint is a separate Rust binary (`[[bin]]` in `Cargo.toml`) that uses the [`vercel_runtime`](https://crates.io/crates/vercel_runtime) crate with its `axum` feature. The entry point follows the pattern:

```rust
#[tokio::main]
async fn main() -> Result<(), Error> {
    // tracing setup …
    let app = service_fn(handler);
    vercel_runtime::run(app).await
}
```

`service_fn` wraps an async handler that receives a standard `http::Request` and returns an `http::Response`. Tokio provides the async runtime inside the serverless sandbox.

Structured logging uses `tracing` + `tracing-subscriber`, emitting JSON-friendly lines that appear in Vercel's function log viewer.

#### `api/quiz.rs` — Quiz generation

- **Method:** `POST /api/quiz`
- **Request body:** `{ "name": "<user>", "mode": "<mode>" }`
- **Response:** `{ "questions": [ { "question", "options": ["A","B","C","D","E"], "correct_index" } ] }`

Steps:
1. Validates `mode` against the four accepted values.
2. Opens a Redis connection via `REDIS_URL`.
3. Fetches the full `vocab` list from Redis (`LRANGE vocab 0 -1`).
4. Shuffles the list with `rand` and takes the first 10 items.
5. For each item, builds the question string and the correct answer according to the mode, then samples 4 random wrong answers from the remaining vocab pool (deduped via `HashSet`).
6. Inserts the correct answer at a random position among the five options.
7. Returns all 10 `Question` objects.

#### `api/history.rs` — Session history

**`GET /api/history?name=<user>&page=<n>`**

- Looks up the Redis list `sessions:<name>` (lowercase).
- Uses `LLEN` to get the total count, then calculates `total_pages = ceil(total / 10)`.
- Fetches the correct 10-item slice with `LRANGE` using computed start/end indices (newest-first within the page).
- Returns:
  ```json
  {
    "sessions": [ { "date", "correct", "total", "mode" } ],
    "page": 1,
    "total_pages": 3,
    "total_count": 27
  }
  ```

**`POST /api/history`**

- **Request body:** `{ "name", "correct", "total", "mode", "date" }`
- Serialises a `Session` struct to JSON and appends it to `sessions:<name>` with `RPUSH`.
- The frontend fires this automatically (fire-and-forget) when the results screen is shown.

### Storage — Redis

Two key namespaces are used:

| Key | Type | Content |
|---|---|---|
| `vocab` | List | One JSON string per vocabulary item: `{ "pinyin", "english", "characters" }` |
| `sessions:<name>` | List | One JSON string per quiz session: `{ "date", "correct", "total", "mode" }` |

Sessions are appended with `RPUSH` (oldest → newest) so that paginating from the tail always yields the most recent entries first.

#### Loading vocabulary

The `load_vocab.py` script seeds the `vocab` key from a TSV file (`Chinese Vocab - Sheet1.tsv`) with columns `Pinyin | English | Characters`. Run it once (or whenever the word list changes):

```bash
pip install redis
REDIS_URL=redis://default:<password>@<host>:6379 python load_vocab.py
```

The script deletes the existing `vocab` key, bulk-loads all rows via a pipeline, and prints a sample of the first three loaded items for verification.

### Authentication — Edge Middleware

`middleware.ts` runs on Vercel's Edge Runtime and intercepts every non-static request before it reaches the functions. It enforces HTTP Basic Auth using credentials read from environment variables:

| Variable | Default | Purpose |
|---|---|---|
| `AUTH_USER` | `admin` | Expected username |
| `AUTH_PASS` | `password` | Expected password |

The middleware splits the `Authorization: Basic …` header on the **first colon only**, so passwords containing `:` are supported.

---

## Environment Variables

| Variable | Required | Description |
|---|---|---|
| `REDIS_URL` | ✅ | Full Redis connection URL, e.g. `redis://default:<pw>@host:6379` or `rediss://…` for TLS |
| `AUTH_USER` | ❌ | HTTP Basic Auth username (default: `admin`) |
| `AUTH_PASS` | ❌ | HTTP Basic Auth password (default: `password`) |

Set these in the Vercel project dashboard under **Settings → Environment Variables**, or in a local `.env` file for development.

---

## Local Development

```bash
# 1. Install the Vercel CLI
npm i -g vercel

# 2. Build Rust binaries and start the local dev server
./dev.sh
```

`dev.sh` runs `cargo build` (debug profile) and then `vercel dev`, which serves `index.html` statically and proxies `/api/*` requests to the locally compiled binaries.

Make sure `REDIS_URL` is set in your environment or in `.env.local` before starting.

---

## Dependencies

| Crate | Purpose |
|---|---|
| `vercel_runtime` | Vercel serverless function adapter with Axum/`service_fn` support |
| `tokio` | Async runtime |
| `redis` | Async Redis client (TLS via `tokio-native-tls-comp`) |
| `serde` / `serde_json` | JSON serialisation |
| `rand` | Shuffling vocab and placing the correct answer at a random position |
| `tracing` / `tracing-subscriber` | Structured logging |
| `http-body-util` | Reading request body bytes |

### Front-end libraries (CDN)

| Library | Purpose |
|---|---|
| Bootstrap 5.3 | Responsive layout, buttons, cards, badges, tables |
| Chart.js 4 | Progress chart (scatter + line, time scale) |
| chartjs-adapter-date-fns | Date/time axis formatting for Chart.js |

