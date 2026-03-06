# TODO (from PLAN.md)

## Legend

- [ ] Pending
- [x] Done

## Phase 1 — Core system

- [x] Expose OIDC CLI/env configuration via clap
  - `WEBSITES_TLS_CERT_PATH`
  - `WEBSITES_TLS_KEY_PATH`
  - `WEBSITES_FRONTEND_URL`
  - `WEBSITES_OIDC_CLIENT_ID`
  - `WEBSITES_OIDC_DISCOVERY_URL`
  - [x] Add `show-config` command
- [x] Make admin UI the default runtime action
- [x] Redirect `/` to `/admin`
- [ ] Implement web authentication + OIDC login/logout flow
  - Requires session store + OIDC integration
- [x] Add admin route set
  - `/admin`
  - `/admin/sites`
  - `/admin/sites/new`
  - `/admin/login`
  - `/admin/logout`
  - `/admin/site/<site_id>/content`
  - `/admin/site/<site_id>/content/<content_id>`
  - `/admin/site/<site_id>/content/<content_id>/source`
  - `/admin/site/<site_id>/content/<content_id>/advanced`
  - `/admin/site/<site_id>/content/<content_id>/revisions`
  - `/admin/site/<site_id>/tags`
  - `/admin/site/<site_id>/assets`
  - `/admin/site/<site_id>/settings`
  - `/admin/site/<site_id>/render`
- [x] Serve admin UI via server-rendered HTML (no SPA dependency)

## Phase 2 — Content features

- [x] Hook admin actions to `audit_event` logging table
- [x] Implement server-rendered site create flow at `/admin/sites/new`
- [x] Implement server-rendered content create flow at `/admin/site/<site_id>/content/new`
- [x] Implement media upload pipeline and derivative generation (thumbnail creation)
- [x] Add source-mode editor flow
- [x] Add scheduled publishing capabilities (beyond `published_at` field)

## Phase 3 — Advanced features

- [ ] Build WordPress import/migration tooling using alias preservation
- [ ] Implement revision diff viewer
- [x] Add content search
- [ ] Add media embedding support in editor

## Template/rendering backlog

- [x] Decide and finalize template engine strategy (PLAN mentions Askama/Tera direction)
- [x] Migrate current fixed-string template rendering to chosen engine
- [ ] Expand template coverage/slots as needed for metadata and edge cases
