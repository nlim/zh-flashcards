# Agent Context: 中文 Flashcards

This repository is a Chinese vocabulary flashcards application deployed on Vercel.

## 🏗 Architecture
- **Frontend**: A single-page application (SPA) using `index.html` with embedded JavaScript and CSS. It uses Bootstrap 5 for styling and Chart.js for progress visualization.
- **Backend**: Rust-based serverless functions deployed as Vercel Functions.
- **Platform**: Vercel.

## 🛠 Tech Stack
- **Frontend**: HTML5, CSS3, JavaScript (Vanilla), Bootstrap 5.
- **Backend**: Rust.
- **Deployment**: Vercel.
- **Data**: Vocab items and user history are managed via Rust API endpoints.

## 📁 Project Structure
- `/api`: Contains the Rust backend functions.
  - `autofill.rs`: AI-powered autofill for vocab items.
  - `history.rs`: Manages user quiz history.
  - `quiz.rs`: Handles quiz logic and question generation.
  - `vocab.rs`: Manages the vocabulary list (CRUD operations).
- `index.html`: The main frontend entry point.
- `vercel.json`: Vercel configuration for routing and rewrites.
- `Cargo.toml`: Rust dependency management.
- `load_vocab.py`: Script for loading/initializing vocabulary data.
- `middleware.ts`: Vercel Edge Middleware for request handling.

## 📡 API Endpoints
- `POST /api/autofill`: Suggests Pinyin and Characters for a given English word.
- `GET/POST/PUT/DELETE /api/vocab`: Manages the vocabulary list.
- `GET/POST /api/history`: Manages quiz history and statistics.
- `GET /api/quiz`: Generates quiz questions based on user preferences.

## 📝 Coding Guidelines
- **Frontend**: Keep JavaScript within `index.html` for simplicity as requested by the architecture. Use Bootstrap classes for responsiveness.
- **Backend**: Ensure Rust functions are optimized for serverless execution. Follow Rust's ownership and borrowing rules.
- **Consistency**: Maintain the existing naming conventions (e.g., `zh_to_en`, `en_to_pinyin`).

## 🚀 Development Workflow
- Use `dev.sh` for local development.
- Deploy to Vercel using the Vercel CLI or Git integration.
