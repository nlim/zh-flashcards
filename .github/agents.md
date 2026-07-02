# Agent Skills

## Verify Rust Serverless Functions Compile

To test that all Vercel serverless functions (Rust binaries) compile correctly, run:

```bash
cargo build --bin vocab --bin quiz --bin history --bin autofill
```

This builds all binaries defined in `Cargo.toml` under `[[bin]]` sections:
- `vocab` → `api/vocab.rs`
- `quiz` → `api/quiz.rs`
- `history` → `api/history.rs`
- `autofill` → `api/autofill.rs`

Fix any compiler errors before pushing, as Vercel will fail to deploy if any binary fails to compile.
