pub(crate) static ADMIN_ACTOR_SUB: &str = "web-admin";
pub(crate) static DEFAULT_TEMPLATE_NAME: &str = "default";
pub(crate) static THUMBNAIL_MAX_SIZE: u32 = 320;

pub static SITE_TEMPLATES_DIR: &str = "./site_templates/";
pub static RENDERED_DIR: &str = "./rendered/";

pub static REQUIRED_TEMPLATES: &[&str] = &[
    "index.html",
    "post.html",
    "page.html",
    "tag.html",
    "rss.xml",
    "atom.xml",
];
