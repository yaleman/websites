use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_init"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Site::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Site::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Site::ShortName).string().not_null())
                    .col(ColumnDef::new(Site::FullTitle).string().not_null())
                    .col(ColumnDef::new(Site::TemplateName).string().not_null())
                    .col(ColumnDef::new(Site::CreatedAt).timestamp().not_null())
                    .col(ColumnDef::new(Site::UpdatedAt).timestamp())
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_site_short_name")
                    .table(Site::Table)
                    .col(Site::ShortName)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(User::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(User::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(User::Subject).string().not_null())
                    .col(ColumnDef::new(User::CreatedAt).timestamp().not_null())
                    .col(ColumnDef::new(User::LastLoginAt).timestamp())
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_subject")
                    .table(User::Table)
                    .col(User::Subject)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(TowerSessions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(TowerSessions::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(TowerSessions::Data).blob().not_null())
                    .col(
                        ColumnDef::new(TowerSessions::ExpiryDate)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(SiteMembership::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SiteMembership::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(SiteMembership::SiteId).uuid().not_null())
                    .col(ColumnDef::new(SiteMembership::UserId).uuid().not_null())
                    .col(
                        ColumnDef::new(SiteMembership::Role)
                            .string()
                            .not_null()
                            .check(
                                Expr::col(SiteMembership::Role)
                                    .is_in(vec!["owner", "editor", "author", "viewer"]),
                            ),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_site_membership_site")
                            .from(SiteMembership::Table, SiteMembership::SiteId)
                            .to(Site::Table, Site::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_site_membership_user")
                            .from(SiteMembership::Table, SiteMembership::UserId)
                            .to(User::Table, User::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_site_membership_site_id")
                    .table(SiteMembership::Table)
                    .col(SiteMembership::SiteId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_site_membership_user_id")
                    .table(SiteMembership::Table)
                    .col(SiteMembership::UserId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_site_membership_site_user")
                    .table(SiteMembership::Table)
                    .col(SiteMembership::SiteId)
                    .col(SiteMembership::UserId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(ContentItem::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ContentItem::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ContentItem::SiteId).uuid().not_null())
                    .col(
                        ColumnDef::new(ContentItem::PageType)
                            .string()
                            .not_null()
                            .check(Expr::col(ContentItem::PageType).is_in(vec!["post", "page"])),
                    )
                    .col(ColumnDef::new(ContentItem::Title).string().not_null())
                    .col(ColumnDef::new(ContentItem::Slug).string().not_null())
                    .col(ColumnDef::new(ContentItem::PageContent).string().not_null())
                    .col(ColumnDef::new(ContentItem::Draft).boolean().not_null())
                    .col(ColumnDef::new(ContentItem::CreatorSub).string().not_null())
                    .col(
                        ColumnDef::new(ContentItem::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ContentItem::LastUpdated).timestamp())
                    .col(ColumnDef::new(ContentItem::PublishedAt).timestamp())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_content_item_site")
                            .from(ContentItem::Table, ContentItem::SiteId)
                            .to(Site::Table, Site::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_content_item_site_id")
                    .table(ContentItem::Table)
                    .col(ContentItem::SiteId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_content_item_page_type")
                    .table(ContentItem::Table)
                    .col(ContentItem::PageType)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(ContentAlias::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ContentAlias::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ContentAlias::ContentId).uuid().not_null())
                    .col(ColumnDef::new(ContentAlias::SiteId).uuid().not_null())
                    .col(ColumnDef::new(ContentAlias::AliasPath).string().not_null())
                    .col(
                        ColumnDef::new(ContentAlias::Kind)
                            .string()
                            .not_null()
                            .check(Expr::col(ContentAlias::Kind).is_in(vec!["primary", "alias"])),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_content_alias_content")
                            .from(ContentAlias::Table, ContentAlias::ContentId)
                            .to(ContentItem::Table, ContentItem::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_content_alias_site")
                            .from(ContentAlias::Table, ContentAlias::SiteId)
                            .to(Site::Table, Site::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_content_alias_content_id")
                    .table(ContentAlias::Table)
                    .col(ContentAlias::ContentId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_content_alias_site_id_alias_path")
                    .table(ContentAlias::Table)
                    .col(ContentAlias::SiteId)
                    .col(ContentAlias::AliasPath)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Tag::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Tag::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Tag::SiteId).uuid().not_null())
                    .col(ColumnDef::new(Tag::Name).string().not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_tag_site")
                            .from(Tag::Table, Tag::SiteId)
                            .to(Site::Table, Site::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_tag_site_id_name")
                    .table(Tag::Table)
                    .col(Tag::SiteId)
                    .col(Tag::Name)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(ContentTag::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ContentTag::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ContentTag::ContentId).uuid().not_null())
                    .col(ColumnDef::new(ContentTag::TagId).uuid().not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_content_tag_content")
                            .from(ContentTag::Table, ContentTag::ContentId)
                            .to(ContentItem::Table, ContentItem::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_content_tag_tag")
                            .from(ContentTag::Table, ContentTag::TagId)
                            .to(Tag::Table, Tag::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_content_tag_content_id")
                    .table(ContentTag::Table)
                    .col(ContentTag::ContentId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_content_tag_tag_id")
                    .table(ContentTag::Table)
                    .col(ContentTag::TagId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_content_tag_content_tag")
                    .table(ContentTag::Table)
                    .col(ContentTag::ContentId)
                    .col(ContentTag::TagId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(ContentRevision::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ContentRevision::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ContentRevision::ContentId).uuid().not_null())
                    .col(ColumnDef::new(ContentRevision::SiteId).uuid().not_null())
                    .col(
                        ColumnDef::new(ContentRevision::RevisionNumber)
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ContentRevision::Title).string().not_null())
                    .col(ColumnDef::new(ContentRevision::Slug).string().not_null())
                    .col(
                        ColumnDef::new(ContentRevision::PageContent)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ContentRevision::Draft).boolean().not_null())
                    .col(
                        ColumnDef::new(ContentRevision::PageType)
                            .string()
                            .not_null()
                            .check(
                                Expr::col(ContentRevision::PageType).is_in(vec!["post", "page"]),
                            ),
                    )
                    .col(
                        ColumnDef::new(ContentRevision::EditorSub)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ContentRevision::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_content_revision_content")
                            .from(ContentRevision::Table, ContentRevision::ContentId)
                            .to(ContentItem::Table, ContentItem::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_content_revision_site")
                            .from(ContentRevision::Table, ContentRevision::SiteId)
                            .to(Site::Table, Site::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_content_revision_content_id")
                    .table(ContentRevision::Table)
                    .col(ContentRevision::ContentId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_content_revision_content_revision")
                    .table(ContentRevision::Table)
                    .col(ContentRevision::ContentId)
                    .col(ContentRevision::RevisionNumber)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(ContentRevisionAlias::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ContentRevisionAlias::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ContentRevisionAlias::RevisionId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ContentRevisionAlias::AliasPath)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ContentRevisionAlias::Kind)
                            .string()
                            .not_null()
                            .check(
                                Expr::col(ContentRevisionAlias::Kind)
                                    .is_in(vec!["primary", "alias"]),
                            ),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_content_revision_alias_revision")
                            .from(
                                ContentRevisionAlias::Table,
                                ContentRevisionAlias::RevisionId,
                            )
                            .to(ContentRevision::Table, ContentRevision::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_content_revision_alias_revision_id")
                    .table(ContentRevisionAlias::Table)
                    .col(ContentRevisionAlias::RevisionId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(ContentRevisionTag::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ContentRevisionTag::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ContentRevisionTag::RevisionId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ContentRevisionTag::TagId).uuid().not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_content_revision_tag_revision")
                            .from(ContentRevisionTag::Table, ContentRevisionTag::RevisionId)
                            .to(ContentRevision::Table, ContentRevision::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_content_revision_tag_tag")
                            .from(ContentRevisionTag::Table, ContentRevisionTag::TagId)
                            .to(Tag::Table, Tag::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_content_revision_tag_revision_id")
                    .table(ContentRevisionTag::Table)
                    .col(ContentRevisionTag::RevisionId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_content_revision_tag_revision_tag")
                    .table(ContentRevisionTag::Table)
                    .col(ContentRevisionTag::RevisionId)
                    .col(ContentRevisionTag::TagId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Asset::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Asset::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Asset::SiteId).uuid().not_null())
                    .col(ColumnDef::new(Asset::UploaderSub).string().not_null())
                    .col(ColumnDef::new(Asset::OriginalFilename).string().not_null())
                    .col(ColumnDef::new(Asset::StorageBasename).string().not_null())
                    .col(ColumnDef::new(Asset::MimeType).string().not_null())
                    .col(ColumnDef::new(Asset::ByteLength).integer().not_null())
                    .col(ColumnDef::new(Asset::Width).integer())
                    .col(ColumnDef::new(Asset::Height).integer())
                    .col(ColumnDef::new(Asset::CreatedAt).timestamp().not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_asset_site")
                            .from(Asset::Table, Asset::SiteId)
                            .to(Site::Table, Site::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_asset_site_id")
                    .table(Asset::Table)
                    .col(Asset::SiteId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AssetVariant::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AssetVariant::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(AssetVariant::AssetId).uuid().not_null())
                    .col(
                        ColumnDef::new(AssetVariant::VariantKind)
                            .string()
                            .not_null()
                            .check(
                                Expr::col(AssetVariant::VariantKind)
                                    .is_in(vec!["original", "thumbnail"]),
                            ),
                    )
                    .col(ColumnDef::new(AssetVariant::Filename).string().not_null())
                    .col(ColumnDef::new(AssetVariant::MimeType).string().not_null())
                    .col(
                        ColumnDef::new(AssetVariant::ByteLength)
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(AssetVariant::Width).integer())
                    .col(ColumnDef::new(AssetVariant::Height).integer())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_asset_variant_asset")
                            .from(AssetVariant::Table, AssetVariant::AssetId)
                            .to(Asset::Table, Asset::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_asset_variant_asset_id")
                    .table(AssetVariant::Table)
                    .col(AssetVariant::AssetId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_asset_variant_asset_kind")
                    .table(AssetVariant::Table)
                    .col(AssetVariant::AssetId)
                    .col(AssetVariant::VariantKind)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(AuditEvent::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AuditEvent::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(AuditEvent::SiteId).uuid())
                    .col(ColumnDef::new(AuditEvent::ActorSub).string().not_null())
                    .col(ColumnDef::new(AuditEvent::EventType).string().not_null())
                    .col(ColumnDef::new(AuditEvent::EntityType).string().not_null())
                    .col(ColumnDef::new(AuditEvent::EntityId).string().not_null())
                    .col(ColumnDef::new(AuditEvent::CreatedAt).timestamp().not_null())
                    .col(ColumnDef::new(AuditEvent::PayloadJson).json())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_audit_event_site")
                            .from(AuditEvent::Table, AuditEvent::SiteId)
                            .to(Site::Table, Site::Id)
                            .on_delete(ForeignKeyAction::SetNull),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_audit_event_site_id_created_at")
                    .table(AuditEvent::Table)
                    .col(AuditEvent::SiteId)
                    .col(AuditEvent::CreatedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(AuditEvent::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(AssetVariant::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(Asset::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(ContentRevisionTag::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(ContentRevisionAlias::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(ContentRevision::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(ContentTag::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(Tag::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(ContentAlias::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(ContentItem::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(SiteMembership::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(TowerSessions::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(User::Table).if_exists().to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Site::Table).if_exists().to_owned())
            .await?;
        Ok(())
    }
}

#[derive(Iden)]
enum Site {
    Table,
    Id,
    ShortName,
    FullTitle,
    TemplateName,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
enum User {
    Table,
    Id,
    Subject,
    CreatedAt,
    LastLoginAt,
}

#[derive(Iden)]
enum TowerSessions {
    Table,
    Id,
    Data,
    ExpiryDate,
}

#[derive(Iden)]
enum SiteMembership {
    Table,
    Id,
    SiteId,
    UserId,
    Role,
}

#[derive(Iden)]
enum ContentItem {
    Table,
    Id,
    SiteId,
    PageType,
    Title,
    Slug,
    PageContent,
    Draft,
    CreatorSub,
    CreatedAt,
    LastUpdated,
    PublishedAt,
}

#[derive(Iden)]
enum ContentAlias {
    Table,
    Id,
    ContentId,
    SiteId,
    AliasPath,
    Kind,
}

#[derive(Iden)]
enum Tag {
    Table,
    Id,
    SiteId,
    Name,
}

#[derive(Iden)]
enum ContentTag {
    Table,
    Id,
    ContentId,
    TagId,
}

#[derive(Iden)]
enum ContentRevision {
    Table,
    Id,
    ContentId,
    SiteId,
    RevisionNumber,
    Title,
    Slug,
    PageContent,
    Draft,
    PageType,
    EditorSub,
    CreatedAt,
}

#[derive(Iden)]
enum ContentRevisionAlias {
    Table,
    Id,
    RevisionId,
    AliasPath,
    Kind,
}

#[derive(Iden)]
enum ContentRevisionTag {
    Table,
    Id,
    RevisionId,
    TagId,
}

#[derive(Iden)]
enum Asset {
    Table,
    Id,
    SiteId,
    UploaderSub,
    OriginalFilename,
    StorageBasename,
    MimeType,
    ByteLength,
    Width,
    Height,
    CreatedAt,
}

#[derive(Iden)]
enum AssetVariant {
    Table,
    Id,
    AssetId,
    VariantKind,
    Filename,
    MimeType,
    ByteLength,
    Width,
    Height,
}

#[derive(Iden)]
enum AuditEvent {
    Table,
    Id,
    SiteId,
    ActorSub,
    EventType,
    EntityType,
    EntityId,
    CreatedAt,
    PayloadJson,
}
