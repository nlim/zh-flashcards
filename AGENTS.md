# Agent Context: 中文 Flashcards

This repository is a browser-based Chinese vocabulary quiz application deployed on Vercel.

## 🏗 Architecture
- **Frontend**: A single-page application (SPA) using `index.html` with embedded JavaScript and CSS. It uses Bootstrap 5 for styling and Chart.js 4 (with `chartjs-adapter-date-fns`) for progress visualization. Served via Vercel CDN.
- **Backend**: Rust-based serverless functions deployed as Vercel Functions. Each endpoint is a separate Rust binary using the `vercel_runtime` crate with `axum` features.
- **Data Layer**: Redis (Upstash or compatible). Used for vocabulary storage, user session history, and performance tracking.
- **Platform**: Vercel.

## 🛠 Tech Stack
- **Frontend**: HTML5, CSS3, JavaScript (Vanilla), Bootstrap 5, Chart.js 4.
- **Backend**: Rust (Tokio runtime, Axum, Redis crate, Serde).
- **Deployment**: Vercel (Edge Middleware for Auth, Serverless Functions for API).
- **Authentication**: HTTP Basic Auth enforced via Vercel Edge Middleware (`middleware.ts`).

## 📁 Project Structure
- `/api`: Contains the Rust backend functions.
  - `autofill.rs`: AI-powered autofill for vocab items.
  - `history.rs`: Manages user quiz history (`sessions:<name>` and `perf:<name>:<mode>` in Redis).
  - `quiz.rs`: Handles quiz logic, including "Normal", "Weak spots" (≥ 25% wrong), and "Never seen" modes.
  - `vocab.rs`: Manages the vocabulary list (CRUD operations).
- `index.html`: The main frontend entry point.
- `vercel.json`: Vercel configuration for routing and rewrites ( `/` $\to$ `index.html`).
- `Cargo.toml`: Rust dependency management (configured with `lto = "fat"` and `codegen-units = 1` for production).
- `load_vocab.py`: Python script to seed the Redis `vocab` list from a TSV file.
- `middleware.ts`: Vercel Edge Middleware enforcing Basic Auth.

## 📡 API Endpoints & Redis Schema
### Endpoints
- `POST /api/autofill`: Suggests Pinyin and Characters for a given English word.
- `GET/POST/PUT/DELETE /api/vocab`: Manages the vocabulary list.
- `GET /api/history?name=<user>&page=<n>`: Fetches paginated history (10 per page).
- `POST /api/history`: Records session results and individual question performance.
- `POST /api/quiz`: Generates 10 questions. Supports `trouble` (Weak spots) and `fresh` (Never seen) flags.

### Redis Key Namespaces
- `vocab` (List): JSON strings of `{ "pinyin", "english", "characters" }`.
- `sessions:<name>` (List): JSON strings of `{ "date", "correct", "total", "mode" }`.
- `perf:<name>:<mode>` (List): JSON strings of `{ "date", "vocab", "correct" }`.

## 📝 Coding Guidelines
- **Frontend**: Keep JavaScript within `index.html`. Use Bootstrap classes for responsiveness. Chart.js data is processed client-side by fetching all history pages in parallel.
- **Backend**: Rust functions must be optimized for serverless cold-starts. 
  - **Important**: `ThreadRng` is `!Send` and cannot be held across `.await` points. Fetch Redis data *before* creating the RNG.
  - Use `tracing` for structured JSON logging.
- **Consistency**: Maintain mode naming: `zh_to_en`, `pinyin_to_en`, `en_to_pinyin`, `en_to_zh`.

## 🚀 Development Workflow
- **Local Dev**: Run `./dev.sh` (runs `cargo build` and `vercel dev`).
- **Data Seeding**: Use `REDIS_URL=... python load_vocab.py`.
- **Env Vars**: Requires `REDIS_URL`, `AUTH_USER`, and `AUTH_PASS`.

## ✅ Verifying Rust Serverless Functions Compile

Before pushing, verify all Vercel serverless function binaries compile:

```bash
cargo build --bin vocab --bin quiz --bin history --bin autofill
# Or run a full build analysis
cargo build analyze
```

This covers all `[[bin]]` entries in `Cargo.toml`. Fix any compiler errors before pushing — Vercel will fail to deploy if any binary fails to compile.
