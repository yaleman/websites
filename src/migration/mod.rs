use sea_orm_migration::prelude::*;

mod m0001_init;
mod m0002_add_email;
mod m0003_add_admin;
mod m0004_add_display_name;
mod m0005_revision_children_content_fk;
mod m0006_add_settings_and_api_tokens;
mod m0007_add_theme_registry;
mod m0008_add_site_publish_workflow;
mod m0009_add_rsync_publish_method;
mod m0010_add_publish_on_render;
mod m0011_add_theme_ssh_key;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m0001_init::Migration),
            Box::new(m0002_add_email::Migration),
            Box::new(m0003_add_admin::Migration),
            Box::new(m0004_add_display_name::Migration),
            Box::new(m0005_revision_children_content_fk::Migration),
            Box::new(m0006_add_settings_and_api_tokens::Migration),
            Box::new(m0007_add_theme_registry::Migration),
            Box::new(m0008_add_site_publish_workflow::Migration),
            Box::new(m0009_add_rsync_publish_method::Migration),
            Box::new(m0010_add_publish_on_render::Migration),
            Box::new(m0011_add_theme_ssh_key::Migration),
        ]
    }
}
