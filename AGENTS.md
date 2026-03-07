# Ruminate Agent Rules

## Required Process

- Use Cargo tooling for dependency changes only (`cargo add`, `cargo remove`).
- Do not use the SeaORM CLI.
- All DB schema changes must be made through migration files in the db crate.
- Any change to system design must include an update to this file.
- A task is not complete until `mise check` passes.
- Commit changes once a user request is confirmed complete.
- Tests must use run-specific temporary directories for filesystem-emulated state; use OS temp locations (for example /tmp via tempfile/mkdtemp) and never create test temp workspaces inside the repository tree. For database-backed tests, prefer in-memory databases when feasible. Per test, use one fresh instance of every mutable dependency (filesystem roots, database, and service process) and never reuse local persistent state.
- Never use inline JavaScript, inline TypeScript, or inline CSS in frontend files; use external `.ts/.tsx/.js` and `.css` assets only.
- Fill page data primarily through Askama templates rendered on the server; frontend JavaScript should be used for client-side actions and progressive enhancement, not initial content hydration when template rendering can provide the content.
- Runtime logging must use `tracing` only, include timestamps, and write to stdout.
- In async runtime codepaths, use `tokio::fs` instead of `std::fs` for filesystem operations.
- Prefer workspace-level dependency declarations and `workspace = true` member usage to keep crate versions aligned.
- Binaries should parse CLI arguments and environment variables via `clap`.
- All askama templates should derive `askama_web::WebTemplate` so we don't have to manually implement `IntoResponse`

## System Design Updates

- 2026-03-07: Replaced raw SQL schema bootstrapping with SeaORM migrations (`sea-orm-migration`) and set SQLite foreign keys via the connection URL.
