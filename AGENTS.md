# Agent Rules

This is a web-based system for managing content of websites.

## System Overview

The system manages site metadata, content, tags, assets, and memberships in a SQLite database through SeaORM. Content is stored as Markdown and can be rendered into static output using templates and media assets, enabling a publish step that generates static files. The admin interface is primarily server-rendered with Askama templates from the `templates/` directory, with JavaScript used for the rich-text editor and other progressive enhancements. Site publishing and content previews use Tera templates loaded from the `site_templates/` directory, keeping site-facing rendering separate from admin UI rendering. Preview responses must rewrite root-relative site template asset paths like `/assets/...` to the authenticated admin preview asset route, which serves files from `site_templates/<template>/assets/` without exposing them through the global admin asset directory. Authentication is handled via session storage with optional OIDC integration for login, and operational workflows are available through a CLI for initialization, imports, and admin tasks.

## Required Process

- Use Cargo tooling for dependency changes only (`cargo add`, `cargo remove`).
- Do not use the SeaORM CLI.
- All DB schema changes must be made through migration files in the db crate.
- If more than one database modification is needed, start a transaction and use it, so that failures roll back all changes.
- Any change to system design must update docs/src/System-Design-Updates.md.
- A task is not complete until `mise check` passes.
- Commit changes once a user request is confirmed complete.
- Tests must use run-specific temporary directories for filesystem-emulated state; use OS temp locations (for example /tmp via tempfile/mkdtemp) and never create test temp workspaces inside the repository tree. For database-backed tests, prefer in-memory databases when feasible. Per test, use one fresh instance of every mutable dependency (filesystem roots, database, and service process) and never reuse local persistent state.
- Never use inline JavaScript, inline TypeScript, or inline CSS in frontend files; use external `.ts/.tsx/.js` and `.css` assets only.
- Fill page data primarily through Askama templates rendered on the server; frontend JavaScript should be used for client-side actions and progressive enhancement, not initial content hydration when template rendering can provide the content.
- Runtime logging must use `tracing` only, include timestamps, and write to stdout.
- In async runtime codepaths, use `tokio::fs` instead of `std::fs` for filesystem operations.
- Binaries should parse CLI arguments and environment variables via `clap`.
- All askama templates should derive `askama_web::WebTemplate` so we don't have to manually implement `IntoResponse`
- All admin templates should be based on `base_template.html` to keep navigation/UI consistent.
- Each admin view should have its own template file (no shared admin view template).

@docs/src/System-Design-Updates.md
