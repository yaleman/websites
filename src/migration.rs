pub const SCHEMA_SQL: &[&str] = &[
    "PRAGMA foreign_keys = ON;",
    "CREATE TABLE IF NOT EXISTS site (
        id TEXT PRIMARY KEY,
        short_name TEXT NOT NULL UNIQUE,
        full_title TEXT NOT NULL,
        template_name TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );",
    "CREATE TABLE IF NOT EXISTS \"user\" (
        id TEXT PRIMARY KEY,
        subject TEXT NOT NULL UNIQUE,
        created_at TEXT NOT NULL,
        last_login_at TEXT
    );",
    "CREATE TABLE IF NOT EXISTS site_membership (
        id TEXT PRIMARY KEY,
        site_id TEXT NOT NULL REFERENCES site(id) ON DELETE CASCADE,
        user_id TEXT NOT NULL REFERENCES \"user\"(id) ON DELETE CASCADE,
        role TEXT NOT NULL CHECK(role IN ('owner', 'editor', 'author', 'viewer')),
        UNIQUE(site_id, user_id)
    );",
    "CREATE INDEX IF NOT EXISTS idx_site_membership_site_id ON site_membership(site_id);",
    "CREATE INDEX IF NOT EXISTS idx_site_membership_user_id ON site_membership(user_id);",
    "CREATE TABLE IF NOT EXISTS content_item (
        id TEXT PRIMARY KEY,
        site_id TEXT NOT NULL REFERENCES site(id) ON DELETE CASCADE,
        page_type TEXT NOT NULL CHECK(page_type IN ('post', 'page')),
        title TEXT NOT NULL,
        slug TEXT NOT NULL,
        page_content TEXT NOT NULL,
        draft INTEGER NOT NULL CHECK(draft IN (0, 1)),
        creator_sub TEXT NOT NULL,
        created_at TEXT NOT NULL,
        last_updated TEXT NOT NULL,
        published_at TEXT
    );",
    "CREATE INDEX IF NOT EXISTS idx_content_item_site_id ON content_item(site_id);",
    "CREATE INDEX IF NOT EXISTS idx_content_item_page_type ON content_item(page_type);",
    "CREATE TABLE IF NOT EXISTS content_alias (
        id TEXT PRIMARY KEY,
        content_id TEXT NOT NULL REFERENCES content_item(id) ON DELETE CASCADE,
        site_id TEXT NOT NULL REFERENCES site(id) ON DELETE CASCADE,
        alias_path TEXT NOT NULL,
        kind TEXT NOT NULL CHECK(kind IN ('primary', 'alias')),
        UNIQUE(site_id, alias_path)
    );",
    "CREATE INDEX IF NOT EXISTS idx_content_alias_content_id ON content_alias(content_id);",
    "CREATE INDEX IF NOT EXISTS idx_content_alias_site_id_alias_path ON content_alias(site_id, alias_path);",
    "CREATE TABLE IF NOT EXISTS tag (
        id TEXT PRIMARY KEY,
        site_id TEXT NOT NULL REFERENCES site(id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        UNIQUE(site_id, name)
    );",
    "CREATE INDEX IF NOT EXISTS idx_tag_site_id_name ON tag(site_id, name);",
    "CREATE TABLE IF NOT EXISTS content_tag (
        id TEXT PRIMARY KEY,
        content_id TEXT NOT NULL REFERENCES content_item(id) ON DELETE CASCADE,
        tag_id TEXT NOT NULL REFERENCES tag(id) ON DELETE CASCADE,
        UNIQUE(content_id, tag_id)
    );",
    "CREATE INDEX IF NOT EXISTS idx_content_tag_content_id ON content_tag(content_id);",
    "CREATE INDEX IF NOT EXISTS idx_content_tag_tag_id ON content_tag(tag_id);",
    "CREATE TABLE IF NOT EXISTS content_revision (
        id TEXT PRIMARY KEY,
        content_id TEXT NOT NULL REFERENCES content_item(id) ON DELETE CASCADE,
        site_id TEXT NOT NULL REFERENCES site(id) ON DELETE CASCADE,
        revision_number INTEGER NOT NULL,
        title TEXT NOT NULL,
        slug TEXT NOT NULL,
        page_content TEXT NOT NULL,
        draft INTEGER NOT NULL CHECK(draft IN (0, 1)),
        page_type TEXT NOT NULL CHECK(page_type IN ('post', 'page')),
        editor_sub TEXT NOT NULL,
        created_at TEXT NOT NULL,
        UNIQUE(content_id, revision_number)
    );",
    "CREATE INDEX IF NOT EXISTS idx_content_revision_content_id ON content_revision(content_id);",
    "CREATE TABLE IF NOT EXISTS content_revision_alias (
        id TEXT PRIMARY KEY,
        revision_id TEXT NOT NULL REFERENCES content_revision(id) ON DELETE CASCADE,
        alias_path TEXT NOT NULL,
        kind TEXT NOT NULL CHECK(kind IN ('primary', 'alias'))
    );",
    "CREATE INDEX IF NOT EXISTS idx_content_revision_alias_revision_id ON content_revision_alias(revision_id);",
    "CREATE TABLE IF NOT EXISTS asset (
        id TEXT PRIMARY KEY,
        site_id TEXT NOT NULL REFERENCES site(id) ON DELETE CASCADE,
        uploader_sub TEXT NOT NULL,
        original_filename TEXT NOT NULL,
        storage_basename TEXT NOT NULL,
        mime_type TEXT NOT NULL,
        byte_length INTEGER NOT NULL,
        width INTEGER,
        height INTEGER,
        created_at TEXT NOT NULL
    );",
    "CREATE INDEX IF NOT EXISTS idx_asset_site_id ON asset(site_id);",
    "CREATE TABLE IF NOT EXISTS asset_variant (
        id TEXT PRIMARY KEY,
        asset_id TEXT NOT NULL REFERENCES asset(id) ON DELETE CASCADE,
        variant_kind TEXT NOT NULL CHECK(variant_kind IN ('original', 'thumbnail')),
        filename TEXT NOT NULL,
        mime_type TEXT NOT NULL,
        byte_length INTEGER NOT NULL,
        width INTEGER,
        height INTEGER,
        UNIQUE(asset_id, variant_kind)
    );",
    "CREATE INDEX IF NOT EXISTS idx_asset_variant_asset_id ON asset_variant(asset_id);",
    "CREATE TABLE IF NOT EXISTS audit_event (
        id TEXT PRIMARY KEY,
        site_id TEXT REFERENCES site(id) ON DELETE SET NULL,
        actor_sub TEXT NOT NULL,
        event_type TEXT NOT NULL,
        entity_type TEXT NOT NULL,
        entity_id TEXT NOT NULL,
        created_at TEXT NOT NULL,
        payload_json TEXT
    );",
    "CREATE INDEX IF NOT EXISTS idx_audit_event_site_id_created_at ON audit_event(site_id, created_at);",
];
