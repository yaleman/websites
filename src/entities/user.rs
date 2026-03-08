use chrono::DateTime as ChronoDateTime;
use chrono::Utc;
use sea_orm::ActiveValue::Set;
use sea_orm::IntoActiveModel;
use sea_orm::entity::prelude::*;

use crate::errors::SiteError;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "user")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub subject: String,
    pub created_at: ChronoDateTime<Utc>,
    pub last_login_at: Option<ChronoDateTime<Utc>>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// Creates a user record and returns the persisted row.
pub async fn create_user<C: ConnectionTrait>(db: &C, subject: &str) -> Result<Model, SiteError> {
    let model = ActiveModel {
        id: Set(Uuid::now_v7()),
        subject: Set(subject.to_string()),
        created_at: Set(Utc::now()),
        last_login_at: Set(None),
    };

    model.insert(db).await.map_err(|error| error.into())
}

/// Ensures a user exists and updates last_login_at.
pub async fn upsert_user_login<C: ConnectionTrait>(
    db: &C,
    subject: &str,
) -> Result<Model, SiteError> {
    let existing = Entity::find()
        .filter(Column::Subject.eq(subject.to_string()))
        .one(db)
        .await
        .map_err(SiteError::from)?;

    if let Some(existing) = existing {
        let mut active = existing.into_active_model();
        active.last_login_at = Set(Some(Utc::now()));
        active.update(db).await.map_err(SiteError::from)
    } else {
        ActiveModel {
            id: Set(Uuid::now_v7()),
            subject: Set(subject.to_string()),
            created_at: Set(Utc::now()),
            last_login_at: Set(Some(Utc::now())),
        }
        .insert(db)
        .await
        .map_err(SiteError::from)
    }
}
