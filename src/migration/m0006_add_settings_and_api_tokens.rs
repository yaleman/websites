use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0006_add_settings_and_api_tokens"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Settings::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Settings::Key)
                            .string()
                            .not_null()
                            .primary_key()
                            .check(Expr::col(Settings::Key).is_in(vec!["jwt_hs256_secret"])),
                    )
                    .col(ColumnDef::new(Settings::ValueJson).json().not_null())
                    .col(ColumnDef::new(Settings::UpdatedAt).timestamp().not_null())
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(UserApiToken::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(UserApiToken::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(UserApiToken::UserId).uuid().not_null())
                    .col(
                        ColumnDef::new(UserApiToken::IssuedByUserId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(UserApiToken::Label).string().not_null())
                    .col(ColumnDef::new(UserApiToken::JwtId).string().not_null())
                    .col(ColumnDef::new(UserApiToken::GrantsJson).json())
                    .col(
                        ColumnDef::new(UserApiToken::CreatedAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(ColumnDef::new(UserApiToken::LastUsedAt).timestamp())
                    .col(
                        ColumnDef::new(UserApiToken::InactiveExpiresAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(ColumnDef::new(UserApiToken::RevokedAt).timestamp())
                    .col(ColumnDef::new(UserApiToken::RevokedByUserId).uuid())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_user_api_token_user")
                            .from(UserApiToken::Table, UserApiToken::UserId)
                            .to(User::Table, User::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_user_api_token_issued_by_user")
                            .from(UserApiToken::Table, UserApiToken::IssuedByUserId)
                            .to(User::Table, User::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_user_api_token_revoked_by_user")
                            .from(UserApiToken::Table, UserApiToken::RevokedByUserId)
                            .to(User::Table, User::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_api_token_user_id")
                    .table(UserApiToken::Table)
                    .col(UserApiToken::UserId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_api_token_jwt_id")
                    .table(UserApiToken::Table)
                    .col(UserApiToken::JwtId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_api_token_inactive_expires_at")
                    .table(UserApiToken::Table)
                    .col(UserApiToken::InactiveExpiresAt)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(UserApiToken::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(Settings::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum Settings {
    Table,
    Key,
    ValueJson,
    UpdatedAt,
}

#[derive(Iden)]
enum UserApiToken {
    Table,
    Id,
    UserId,
    IssuedByUserId,
    Label,
    JwtId,
    GrantsJson,
    CreatedAt,
    LastUsedAt,
    InactiveExpiresAt,
    RevokedAt,
    RevokedByUserId,
}

#[derive(Iden)]
enum User {
    Table,
    Id,
}
