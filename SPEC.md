# Rust Static Site Management Platform — System Specification

## Overview

This document specifies the architecture, data model, and operational behavior of a Rust-based website management system designed for **authoring and managing content which renders to static HTML output**.

The platform prioritizes:

- deterministic static rendering
- simple operational deployment
- HTML-first administration UI
- full migration capability from legacy CMS systems (notably WordPress)
- strict separation between canonical database content and rendered output
- predictable filesystem output

The system functions as a **content authoring and management control plane**, while the rendered output is intended to be served by any static hosting solution.

Examples:

- nginx
- S3 static hosting
- CDN-based deployments
- rsync deployments
- object storage

The management platform **never reads rendered content as input**. The SQLite database is the canonical source of truth.

---

# Core Design Principles

## Static First

The platform does **not dynamically render public sites**. All content is rendered to disk.

Public sites consist only of:

- HTML
- CSS
- images
- static assets
- feeds

No runtime server code is required.

---

## HTML-First Administration UI

The administration interface avoids SPA architecture.

Characteristics:

- server-rendered HTML pages
- normal HTTP navigation
- form submissions using POST/redirect/GET
- minimal JavaScript
- progressive enhancement only where required

This ensures:

- mobile compatibility
- low complexity
- reliability
- accessibility
- easy debugging

---

## Database as Canonical Source

All content is stored in SQLite.

Rendered output is always derived from the database.

The system **never parses rendered output**.

```

database → render pipeline → ./rendered/

```

---

## Root-Relative Public URLs

All generated site links are root-relative.

Examples:

```

/about/
/2026/03/06/post-title/
/tags/rust/
/media/images/foo.jpg

```

No relative traversal paths (`../../`) are generated.

This simplifies templates and prevents path calculation errors.

---

## UUIDv7 Identifiers

All persistent entities use **UUIDv7** identifiers.

Reasons:

- sortable
- distributed-safe
- avoids auto-increment exposure
- predictable ordering

---

# Technology Stack

## Backend

Rust

Framework components:

- **Axum** (HTTP framework)
- **Tower middleware**
- **tower-sessions-sqlx-store** (session management)
- **openidconnect crate** (OIDC authentication)
- **SeaORM** (database ORM)
- **SQLite** (primary database)

---

## Frontend

Frontend assets are written in:

- TypeScript
- HTML templates
- minimal JavaScript

Bundling:

- **Rspack**

The admin interface is primarily server-rendered.

---

## Template Engine

**Tera**

Templates are stored at:

```text

./site_templates/<template_name>/

```

The platform ships with a default template.

Templates render:

- posts
- pages
- indexes
- tag archives
- RSS
- Atom feeds

---

## Markdown Rendering

Content is stored as Markdown.

Rendering uses:

```

markdown crate

```

Raw HTML inside Markdown is allowed to support imports from existing sites.

---

# Filesystem Layout

```

project-root/
├─ database.sqlite
├─ admin_templates/ 
├─ site_templates/
│  ├─ default/
│  │  ├─ post.html
│  │  ├─ page.html
│  │  ├─ index.html
│  │  ├─ tag.html
│  │  ├─ rss.xml
│  │  ├─ atom.xml
│  │  ├─ partials/
│  │  └─ assets/
│  └─ other-template/
│
├─ rendered/
│  └─ <site_short_name>/
│
├─ uploads/
│  └─ media-storage/
│
└─ admin-ui-assets/

```

---

# Database Schema

## site

Represents a managed website.

```

id              uuidv7 PRIMARY KEY
short_name      text UNIQUE
full_title      text
template_name   text
created_at      datetime
updated_at      datetime

```

`short_name` determines the filesystem render location.

Example:

```

./rendered/blog/

```

---

## user

Represents an authenticated user.

```

id             uuidv7 PRIMARY KEY
subject        text UNIQUE
created_at     datetime
last_login_at  datetime

```

`subject` corresponds to the OIDC `sub` claim.

---

## site_membership

Maps users to sites with permissions.

```

id        uuidv7 PRIMARY KEY
site_id   uuidv7
user_id   uuidv7
role      enum(owner, editor, author, viewer)

```

Unique constraint:

```

(site_id, user_id)

```

---

## content_item

Represents the current editable version of a page or post.

```

id             uuidv7 PRIMARY KEY
site_id        uuidv7
page_type      enum(post, page)
title          text
slug           text
page_content   text
draft          boolean
creator_sub    text
created_at     datetime
last_updated   datetime
published_at   datetime NULL

```

---

## content_alias

Defines public URL paths for content.

```

id          uuidv7 PRIMARY KEY
content_id  uuidv7
site_id     uuidv7
alias_path  text
kind        enum(primary, alias)

```

Unique constraint:

```

(site_id, alias_path)

```

Example:

```

primary:
/2026/03/06/post-title/

alias:
/wp-content/foo/bar/123/

```

Each alias generates a rendered directory.

---

## tag

```

id       uuidv7 PRIMARY KEY
site_id  uuidv7
name     text

```

Unique:

```

(site_id, name)

```

---

## content_tag

```

id          uuidv7 PRIMARY KEY
content_id  uuidv7
tag_id      uuidv7

```

Unique:

```

(content_id, tag_id)

```

---

## content_revision

Immutable history of content changes.

```

id               uuidv7 PRIMARY KEY
content_id       uuidv7
site_id          uuidv7
revision_number  integer
title            text
slug             text
page_content     text
draft            boolean
page_type        enum(post,page)
editor_sub       text
created_at       datetime

```

---

## content_revision_alias

Snapshots aliases for each revision.

```

id           uuidv7 PRIMARY KEY
revision_id  uuidv7
alias_path   text
kind         enum(primary, alias)

```

---

## asset

Represents uploaded media.

```

id                 uuidv7 PRIMARY KEY
site_id            uuidv7
uploader_sub       text
original_filename  text
storage_basename   text
mime_type          text
byte_length        integer
width              integer NULL
height             integer NULL
created_at         datetime

```

---

## asset_variant

Represents generated derivatives.

```

id            uuidv7 PRIMARY KEY
asset_id      uuidv7
variant_kind  enum(original, thumbnail)
filename      text
mime_type     text
byte_length   integer
width         integer NULL
height        integer NULL

```

Unique:

```

(asset_id, variant_kind)

```

Thumbnail naming:

```

<original_filename>_thumb.<extension>

```

Example:

```

photo.jpg
photo_thumb.jpg

```

---

## audit_event

Records administrative actions.

```

id           uuidv7 PRIMARY KEY
site_id      uuidv7 NULL
actor_sub    text
event_type   text
entity_type  text
entity_id    uuidv7
created_at   datetime
payload_json text

```

---

# Content URL Rules

## Posts

Primary URL format:

```

/<year>/<month>/<day>/<slug>/

```

Rendered path:

```

/2026/03/06/post-title/index.html

```

Filesystem:

```

./rendered/<site>/2026/03/06/post-title/index.html

```

Aliases generate additional rendered directories.

---

## Pages

Pages use their slug directly.

Example:

```

/about/

```

Rendered path:

```

./rendered/<site>/about/index.html

```

Aliases also render additional directories.

---

# Image Management

Images are stored separately from rendered output.

Upload storage:

```

./uploads/media-storage/

```

Generated public files are copied during rendering.

Public path example:

```

/media/images/foo.jpg
/media/images/foo_thumb.jpg

```

Variants:

- original
- thumbnail

Future variants may include:

- retina
- webp
- responsive sizes

---

# Template System

Templates live under:

```

./site_templates/<template_name>/

```

Example:

```

site_templates/default/

```

Structure:

```

post.html
page.html
index.html
tag.html
rss.xml
atom.xml
partials/
assets/

```

---

# Render Pipeline

Publishing a site performs:

1. Load site configuration
2. Load all published content
3. Load tags
4. Load assets
5. Resolve primary URLs
6. Resolve aliases
7. Render content pages
8. Render aliases
9. Render index pages
10. Render tag pages
11. Render RSS feed
12. Render Atom feed
13. Copy template assets
14. Copy media variants
15. Write to temporary directory
16. Atomically swap output

Example:

```

rendered/<site>/.tmp/
→ render
→ rename to rendered/<site>/

```

---

# Admin Interface Structure

## System Zone

```

/admin
/admin/sites
/admin/sites/new
/admin/login
/admin/logout

```

Features:

- site selection
- account information
- authentication

---

## Site Zone

```

/admin/site/<site_id>
/admin/site/<site_id>/content
/admin/site/<site_id>/content/new
/admin/site/<site_id>/content/<content_id>
/admin/site/<site_id>/content/<content_id>/source
/admin/site/<site_id>/content/<content_id>/advanced
/admin/site/<site_id>/content/<content_id>/revisions
/admin/site/<site_id>/tags
/admin/site/<site_id>/assets
/admin/site/<site_id>/settings
/admin/site/<site_id>/render

```

---

# Editor Modes

## Normal Mode

Features:

- rich text editor
- tag selection
- image insertion
- preview
- draft toggle

Markdown hidden.

---

## Source Mode

Displays raw Markdown editor.

---

## Advanced Mode

Displays:

- Markdown
- alias paths
- revision history
- metadata
- URL preview

---

# Revision Model

Every save produces:

1. Update to `content_item`
2. New `content_revision`
3. Snapshot of aliases
4. Snapshot of tags

Revisions are immutable.

---

# RSS and Atom Feeds

Each site generates:

```

/rss.xml
/atom.xml

```

Content included:

- published posts
- ordered by publication date
- limited item count configurable later

---

# WordPress Migration Support

The alias system enables migration without URL breakage.

Example import:

```

old:
[https://example.com/?p=123](https://example.com/?p=123)

alias:
/?p=123

```

or

```

/wp-content/foo/bar/123/

```

These paths are preserved as rendered directories.

---

# Security Model

Authentication:

OIDC

Sessions:

SQLite session store

Authorization:

site_membership roles

Permissions enforced server-side.

---

# Initial Implementation Milestones

## Phase 1

Core system

- authentication
- site management
- content CRUD
- markdown storage
- static rendering
- aliases
- revisions

---

## Phase 2

Content features

- tags
- media uploads
- thumbnails
- RSS / Atom
- audit logging

---

## Phase 3

Advanced features

- WordPress importer
- scheduled publishing
- revision diff
- search
- template customization
- media embedding

---

# System Invariants

These must always remain true:

- The database is canonical.
- Rendered output is derived only.
- Public URLs are root-relative.
- Aliases are explicit records.
- All entities use UUIDv7.
- All site operations are scoped by `site_id`.
- Every content change generates a revision.

```

DB → renderer → static site

```

No reverse data flow.

---

End of specification.
