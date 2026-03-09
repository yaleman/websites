use sea_orm::{Statement, TransactionTrait};
use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0005_revision_children_content_fk"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .transaction::<_, _, DbErr>(|txn| {
                Box::pin(async move {
                    let schema = SchemaManager::new(txn);
                    rebuild_revision_alias_table(&schema, true).await?;
                    rebuild_revision_tag_table(&schema, true).await?;
                    Ok(())
                })
            })
            .await
            .map_err(|error| match error {
                sea_orm::TransactionError::Connection(err) => err,
                sea_orm::TransactionError::Transaction(err) => err,
            })
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .transaction::<_, _, DbErr>(|txn| {
                Box::pin(async move {
                    let schema = SchemaManager::new(txn);
                    rebuild_revision_alias_table(&schema, false).await?;
                    rebuild_revision_tag_table(&schema, false).await?;
                    Ok(())
                })
            })
            .await
            .map_err(|error| match error {
                sea_orm::TransactionError::Connection(err) => err,
                sea_orm::TransactionError::Transaction(err) => err,
            })
    }
}

async fn rebuild_revision_alias_table(
    schema: &SchemaManager<'_>,
    include_content_id: bool,
) -> Result<(), DbErr> {
    let mut create_table = Table::create();
    create_table
        .table(ContentRevisionAliasNew::Table)
        .col(
            ColumnDef::new(ContentRevisionAliasNew::Id)
                .uuid()
                .not_null()
                .primary_key(),
        )
        .col(
            ColumnDef::new(ContentRevisionAliasNew::RevisionId)
                .uuid()
                .not_null(),
        )
        .col(
            ColumnDef::new(ContentRevisionAliasNew::AliasPath)
                .string()
                .not_null(),
        )
        .col(
            ColumnDef::new(ContentRevisionAliasNew::Kind)
                .string()
                .not_null()
                .check(Expr::col(ContentRevisionAliasNew::Kind).is_in(vec!["primary", "alias"])),
        )
        .foreign_key(
            ForeignKey::create()
                .name("fk_content_revision_alias_new_revision")
                .from(
                    ContentRevisionAliasNew::Table,
                    ContentRevisionAliasNew::RevisionId,
                )
                .to(ContentRevision::Table, ContentRevision::Id)
                .on_delete(ForeignKeyAction::Cascade),
        );
    if include_content_id {
        create_table
            .col(
                ColumnDef::new(ContentRevisionAliasNew::ContentId)
                    .uuid()
                    .not_null(),
            )
            .foreign_key(
                ForeignKey::create()
                    .name("fk_content_revision_alias_new_content")
                    .from(
                        ContentRevisionAliasNew::Table,
                        ContentRevisionAliasNew::ContentId,
                    )
                    .to(ContentItem::Table, ContentItem::Id)
                    .on_delete(ForeignKeyAction::Cascade),
            );
    }
    schema.create_table(create_table.to_owned()).await?;

    let backend = schema.get_database_backend();
    let alias_insert_sql = if include_content_id {
        "INSERT INTO content_revision_alias_new (id, revision_id, content_id, alias_path, kind)
         SELECT content_revision_alias.id,
                content_revision_alias.revision_id,
                content_revision.content_id,
                content_revision_alias.alias_path,
                content_revision_alias.kind
         FROM content_revision_alias
         INNER JOIN content_revision ON content_revision.id = content_revision_alias.revision_id"
            .to_string()
    } else {
        "INSERT INTO content_revision_alias_new (id, revision_id, alias_path, kind)
         SELECT id,
                revision_id,
                alias_path,
                kind
         FROM content_revision_alias"
            .to_string()
    };

    schema
        .get_connection()
        .execute(Statement::from_string(backend, alias_insert_sql))
        .await?;

    schema
        .drop_table(Table::drop().table(ContentRevisionAlias::Table).to_owned())
        .await?;
    schema
        .rename_table(
            Table::rename()
                .table(ContentRevisionAliasNew::Table, ContentRevisionAlias::Table)
                .to_owned(),
        )
        .await?;
    schema
        .create_index(
            Index::create()
                .name("idx_content_revision_alias_revision_id")
                .table(ContentRevisionAlias::Table)
                .col(ContentRevisionAlias::RevisionId)
                .to_owned(),
        )
        .await?;
    if include_content_id {
        schema
            .create_index(
                Index::create()
                    .name("idx_content_revision_alias_content_id")
                    .table(ContentRevisionAlias::Table)
                    .col(ContentRevisionAlias::ContentId)
                    .to_owned(),
            )
            .await?;
    }

    Ok(())
}

async fn rebuild_revision_tag_table(
    schema: &SchemaManager<'_>,
    include_content_id: bool,
) -> Result<(), DbErr> {
    let mut create_table = Table::create();
    create_table
        .table(ContentRevisionTagNew::Table)
        .col(
            ColumnDef::new(ContentRevisionTagNew::Id)
                .uuid()
                .not_null()
                .primary_key(),
        )
        .col(
            ColumnDef::new(ContentRevisionTagNew::RevisionId)
                .uuid()
                .not_null(),
        )
        .col(
            ColumnDef::new(ContentRevisionTagNew::TagId)
                .uuid()
                .not_null(),
        )
        .foreign_key(
            ForeignKey::create()
                .name("fk_content_revision_tag_new_revision")
                .from(
                    ContentRevisionTagNew::Table,
                    ContentRevisionTagNew::RevisionId,
                )
                .to(ContentRevision::Table, ContentRevision::Id)
                .on_delete(ForeignKeyAction::Cascade),
        )
        .foreign_key(
            ForeignKey::create()
                .name("fk_content_revision_tag_new_tag")
                .from(ContentRevisionTagNew::Table, ContentRevisionTagNew::TagId)
                .to(Tag::Table, Tag::Id)
                .on_delete(ForeignKeyAction::Cascade),
        );
    if include_content_id {
        create_table
            .col(
                ColumnDef::new(ContentRevisionTagNew::ContentId)
                    .uuid()
                    .not_null(),
            )
            .foreign_key(
                ForeignKey::create()
                    .name("fk_content_revision_tag_new_content")
                    .from(
                        ContentRevisionTagNew::Table,
                        ContentRevisionTagNew::ContentId,
                    )
                    .to(ContentItem::Table, ContentItem::Id)
                    .on_delete(ForeignKeyAction::Cascade),
            );
    }
    schema.create_table(create_table.to_owned()).await?;

    let backend = schema.get_database_backend();
    let tag_insert_sql = if include_content_id {
        "INSERT INTO content_revision_tag_new (id, revision_id, content_id, tag_id)
         SELECT content_revision_tag.id,
                content_revision_tag.revision_id,
                content_revision.content_id,
                content_revision_tag.tag_id
         FROM content_revision_tag
         INNER JOIN content_revision ON content_revision.id = content_revision_tag.revision_id"
            .to_string()
    } else {
        "INSERT INTO content_revision_tag_new (id, revision_id, tag_id)
         SELECT id,
                revision_id,
                tag_id
         FROM content_revision_tag"
            .to_string()
    };

    schema
        .get_connection()
        .execute(Statement::from_string(backend, tag_insert_sql))
        .await?;

    schema
        .drop_table(Table::drop().table(ContentRevisionTag::Table).to_owned())
        .await?;
    schema
        .rename_table(
            Table::rename()
                .table(ContentRevisionTagNew::Table, ContentRevisionTag::Table)
                .to_owned(),
        )
        .await?;
    schema
        .create_index(
            Index::create()
                .name("idx_content_revision_tag_revision_id")
                .table(ContentRevisionTag::Table)
                .col(ContentRevisionTag::RevisionId)
                .to_owned(),
        )
        .await?;
    schema
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
    if include_content_id {
        schema
            .create_index(
                Index::create()
                    .name("idx_content_revision_tag_content_id")
                    .table(ContentRevisionTag::Table)
                    .col(ContentRevisionTag::ContentId)
                    .to_owned(),
            )
            .await?;
    }

    Ok(())
}

#[derive(Iden)]
enum ContentItem {
    Table,
    Id,
}

#[derive(Iden)]
enum ContentRevision {
    Table,
    Id,
}

#[derive(Iden)]
enum ContentRevisionAlias {
    Table,
    RevisionId,
    ContentId,
}

#[derive(Iden)]
#[iden = "content_revision_alias_new"]
enum ContentRevisionAliasNew {
    Table,
    Id,
    RevisionId,
    ContentId,
    AliasPath,
    Kind,
}

#[derive(Iden)]
enum ContentRevisionTag {
    Table,
    RevisionId,
    ContentId,
    TagId,
}

#[derive(Iden)]
#[iden = "content_revision_tag_new"]
enum ContentRevisionTagNew {
    Table,
    Id,
    RevisionId,
    ContentId,
    TagId,
}

#[derive(Iden)]
enum Tag {
    Table,
    Id,
}
