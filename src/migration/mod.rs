use sea_orm_migration::prelude::*;

mod m0001_init;
mod m0002_add_email;
mod m0003_add_admin;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m0001_init::Migration),
            Box::new(m0002_add_email::Migration),
            Box::new(m0003_add_admin::Migration),
        ]
    }
}
