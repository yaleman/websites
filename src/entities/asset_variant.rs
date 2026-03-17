use sea_orm::entity::prelude::*;
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, ToSchema)]
#[sea_orm(table_name = "asset_variant")]
#[schema(
    description = "A variant of an asset, such as a resized image.",
    title = "AssetVariant",
    as = crate::entities::asset_variant::Model
)]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub asset_id: Uuid,
    pub variant_kind: String,
    pub filename: String,
    pub mime_type: String,
    pub byte_length: i32,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
