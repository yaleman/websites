# Agent Rules

This is a web-based system for managing content of websites.

## System Overview

The system manages site metadata, content, tags, assets, and memberships in a SQLite database through SeaORM. Content is stored as Markdown and can be rendered into static output using templates and media assets, enabling a publish step that generates static files. The admin interface is primarily server-rendered with Askama templates from the `templates/` directory, with JavaScript used for the rich-text editor and other progressive enhancements. Site publishing and content previews use Tera templates loaded from the `site_templates/` directory, keeping site-facing rendering separate from admin UI rendering. Preview responses must rewrite root-relative site template asset paths like `/assets/...` to the authenticated admin preview asset route, which serves files from `site_templates/<template>/assets/` without exposing them through the global admin asset directory. When the runtime template root is configured to a mounted data directory, preview and render must still fall back to the bundled `site_templates/` tree without depending on the process working directory. Authentication is handled via session storage with optional OIDC integration for login, and operational workflows are available through a CLI for initialization, imports, and admin tasks.

## Required Process

- Use Cargo tooling for dependency changes only (`cargo add`, `cargo remove`).
- Do not use the SeaORM CLI.
- All DB schema changes must be made through migration files in the db crate.
- If more than one database modification is needed, start a transaction and use it, so that failures roll back all changes.
- Any change to system design must update docs/src/System-Design-Updates.md.
- A task is not complete until `mise check` passes.
- Commit changes once a user request is confirmed complete.
- Tests must use run-specific temporary directories for filesystem-emulated state; use OS temp locations (for example /tmp via tempfile/mkdtemp) and never create test temp workspaces inside the repository tree. For database-backed tests, use `crate::db::test_db_start()` to get an in-memory database instance. Per test, use one fresh instance of every mutable dependency (filesystem roots, database, and service process) and never reuse local persistent state.
- Never use inline JavaScript, inline TypeScript, or inline CSS in frontend files; use external `.ts/.tsx/.js` and `.css` assets only.
- Fill page data primarily through Askama templates rendered on the server; frontend JavaScript should be used for client-side actions and progressive enhancement, not initial content hydration when template rendering can provide the content.
- Runtime logging must use `tracing` only, include timestamps, and write to stdout.
- In async runtime codepaths, use `tokio::fs` instead of `std::fs` for filesystem operations.
- Binaries should parse CLI arguments and environment variables via `clap`.
- In Rust code, string errors are a last resort. Do not use `String`, `&str`, or other stringly-typed errors unless a human has explicitly reviewed and approved that case.
- All askama templates should derive `askama_web::WebTemplate` so we don't have to manually implement `IntoResponse`
- All admin templates should be based on `base_template.html` to keep navigation/UI consistent.
- Each admin view should have its own template file (no shared admin view template).
- After any design, UI, or layout-affecting change, visually check the affected page in a browser at desktop and narrow widths before finishing. Confirm headings, tables, forms, cards, and media previews have clear padding inside bordered surfaces and that no text or image touches or overlaps a border. Use Playwright or the in-app browser to capture screenshots; do not rely on code inspection alone.
- Do not rely on bare semantic elements such as `section` to create cards, panels, or surfaces. Use explicit classes like `surface` for page surfaces and explicit bordered card classes for repeated items, so semantic grouping cannot accidentally create nested cards.
- Every UI/layout-affecting change must include automated Playwright layout assertions where practical, not only screenshots. Cover duplicate action bars, accidental nested cards/surfaces, content touching borders, media preview sizing, and horizontal overflow at narrow widths.
@docs/src/System-Design-Updates.md
