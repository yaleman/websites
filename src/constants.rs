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
pub static CUSTOMIZABLE_TEMPLATE_FILES: &[&str] = &[
    "base_template.html",
    "index.html",
    "post.html",
    "page.html",
    "tag.html",
    "rss.xml",
    "atom.xml",
];

pub static SESSION_USER: &str = "user";

pub(crate) static SESSION_OIDC_PKCE_KEY: &str = "oidc_pkce";
pub(crate) static SESSION_OIDC_STATE_KEY: &str = "oidc_state";
pub(crate) static SESSION_OIDC_NONCE_KEY: &str = "oidc_nonce";
