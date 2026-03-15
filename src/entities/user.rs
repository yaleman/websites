use chrono::DateTime as ChronoDateTime;
use chrono::Utc;
use sea_orm::ActiveValue::Set;
use sea_orm::IntoActiveModel;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

use crate::errors::SiteError;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "user")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub subject: String,
    pub created_at: ChronoDateTime<Utc>,
    pub last_login_at: Option<ChronoDateTime<Utc>>,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub admin: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// Creates a user record and returns the persisted row.
pub async fn create_user<C: ConnectionTrait>(
    db: &C,
    subject: &str,
    email: Option<&str>,
    display_name: Option<&str>,
    admin: bool,
) -> Result<Model, SiteError> {
    let model = ActiveModel {
        id: Set(Uuid::now_v7()),
        subject: Set(subject.to_string()),
        created_at: Set(Utc::now()),
        last_login_at: Set(None),
        email: Set(email.map(|e| e.to_string())),
        display_name: Set(display_name.map(|value| value.to_string())),
        admin: Set(admin),
    };

    model.insert(db).await.map_err(|error| error.into())
}

/// Ensures a user exists and updates last_login_at.
pub async fn upsert_user_login<C: ConnectionTrait>(
    db: &C,
    subject: &str,
    email: Option<&str>,
    display_name: Option<&str>,
) -> Result<Model, SiteError> {
    if Entity::find().count(db).await? == 0 {
        // No users exist, create the first user as an admin

        return create_user(db, subject, email, display_name, true).await;
    }

    let existing = Entity::find()
        .filter(Column::Subject.eq(subject.to_string()))
        .one(db)
        .await?;

    if let Some(existing) = existing {
        let mut active = existing.into_active_model();
        active.last_login_at = Set(Some(Utc::now()));
        if let Some(email) = email {
            active.email = Set(Some(email.to_string()));
        }
        if let Some(display_name) = display_name {
            active.display_name = Set(Some(display_name.to_string()));
        }
        active.update(db).await.map_err(SiteError::from)
    } else {
        create_user(db, subject, email, display_name, false).await
    }
}

#[cfg(test)]
mod tests {
    use crate::db::test_db_start;

    use super::*;

    #[tokio::test]
    async fn first_login_creates_admin_with_email() {
        let db = test_db_start().await;

        let user = upsert_user_login(
            &db,
            "first-user",
            Some("first@example.com"),
            Some("First User"),
        )
        .await
        .expect("failed to upsert first user");

        assert!(user.admin);
        assert_eq!(user.email.as_deref(), Some("first@example.com"));
        assert_eq!(user.display_name.as_deref(), Some("First User"));
    }

    #[tokio::test]
    async fn existing_login_refreshes_claims() {
        let db = test_db_start().await;
        let created = create_user(&db, "subject", None, None, false)
            .await
            .expect("failed to create user");
        let updated = upsert_user_login(
            &db,
            "subject",
            Some("subject@example.com"),
            Some("Subject User"),
        )
        .await
        .expect("failed to refresh user login");

        assert_eq!(updated.id, created.id);
        assert_eq!(updated.email.as_deref(), Some("subject@example.com"));
        assert_eq!(updated.display_name.as_deref(), Some("Subject User"));
        assert!(updated.last_login_at.is_some());
    }
}
