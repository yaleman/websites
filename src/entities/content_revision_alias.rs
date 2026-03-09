use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_revision_alias")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub revision_id: Uuid,
    pub content_id: Uuid,
    pub alias_path: String,
    pub kind: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::content_revision::Entity",
        from = "Column::RevisionId",
        to = "super::content_revision::Column::Id",
        on_delete = "Cascade"
    )]
    ContentRevision,
    #[sea_orm(
        belongs_to = "super::content_item::Entity",
        from = "Column::ContentId",
        to = "super::content_item::Column::Id",
        on_delete = "Cascade"
    )]
    ContentItem,
}

impl Related<super::content_revision::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ContentRevision.def()
    }
}

impl Related<super::content_item::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ContentItem.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
