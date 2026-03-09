# Agent Rules

This is a web-based system for managing content of websites.

## System Overview

The system manages site metadata, content, tags, assets, and memberships in a SQLite database through SeaORM. Content is stored as Markdown and can be rendered into static output using templates and media assets, enabling a publish step that generates static files. The admin interface is primarily server-rendered with Askama templates from the `templates/` directory, with JavaScript used for the rich-text editor and other progressive enhancements. Site publishing and content previews use Tera templates loaded from the `site_templates/` directory, keeping site-facing rendering separate from admin UI rendering. Preview responses must rewrite root-relative site template asset paths like `/assets/...` to the authenticated admin preview asset route, which serves files from `site_templates/<template>/assets/` without exposing them through the global admin asset directory. Authentication is handled via session storage with optional OIDC integration for login, and operational workflows are available through a CLI for initialization, imports, and admin tasks.

## Required Process

- Use Cargo tooling for dependency changes only (`cargo add`, `cargo remove`).
- Do not use the SeaORM CLI.
- All DB schema changes must be made through migration files in the db crate.
- If more than one database modification is needed, start a transaction and use it, so that failures roll back all changes.
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
- All admin templates should be based on `base_template.html` to keep navigation/UI consistent.
- Each admin view should have its own template file (no shared admin view template).

## System Design Updates

- 2026-03-07: Replaced raw SQL schema bootstrapping with SeaORM migrations (`sea-orm-migration`) and set SQLite foreign keys via the connection URL.
- 2026-03-08: Documented the rendering split: admin UI templates use Askama from `templates/`, while site previews and published site output use Tera from `site_templates/`.
- 2026-03-08: Site previews now rewrite `/assets/...` references to an authenticated preview asset route that serves files from `site_templates/<template>/assets/`.
- 2026-03-08: Added a global user admin flag that bypasses per-site membership checks for admin routes; site memberships remain limited to viewer/author/editor/owner roles. Published site rendering now copies uploaded media from the resolved runtime upload root instead of assuming a fixed relative path.
- 2026-03-08: Added a user profile admin view keyed by database user UUID. Global admins can view any user profile, and non-admin users can view only their own profile; the view shows stored user details plus site memberships.
- 2026-03-09: Added per-site template overrides stored under the runtime upload root at `.site-template-overrides/<site_id>/`. Site preview and render load these overrides before the shared `site_templates/<template>/` files, and the admin UI currently edits template files only, not template asset directories.
- 2026-03-09: Site memberships can only be granted to users who already exist from a prior login. The admin memberships page now presents an autocomplete picker over known user email/subject values instead of free-form subject entry.
- 2026-03-09: User records now persist the OIDC display-name claim when available, and the admin user profile shows that display name alongside subject and email.
- 2026-03-09: Unmatched routes now render a human-facing 404 page with the requested path and a link back to `/` instead of returning Axum's default plain response body.
- 2026-03-09: Site exports are available through the CLI and an owner-only admin download endpoint as a versioned JSON document containing site-scoped database records plus uploaded media/template-override file metadata, but not file bytes.
