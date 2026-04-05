use super::content::format_optional_datetime;
use super::*;
use serde::{Deserialize, Deserializer};

pub(crate) struct AdminTemplateData {
    pub(crate) page_title: String,
    pub(crate) document_title: String,

    /// Feedback to the user
    pub(crate) page_message: Option<String>,
    /// Render the page message as a toast notification instead of an inline banner.
    pub(crate) page_message_is_toast: bool,
    /// Query parameter to clear after displaying a toast message.
    pub(crate) clear_query_param: Option<String>,
    /// Used when you're in a site context, to link back to the site homepage, e.g. in the header.
    pub(crate) site_id: Option<Uuid>,
    /// Full site title shown in shared admin chrome for site-scoped pages.
    pub(crate) site_full_title: Option<String>,
    /// Whether publishing is configured for this site.
    pub(crate) site_publish_configured: bool,
    /// Extra "actions" links in a secondary navbar, e.g. "New site", "Back to sites", etc.
    pub(crate) links: Vec<AdminLink>,
    pub(crate) nav_search_action: String,
    pub(crate) nav_search_value: String,
    pub(crate) hide_nav: bool,
}

impl AdminTemplateData {
    pub fn new(page_title: &str) -> Self {
        Self {
            document_title: page_title.to_string(),
            page_title: page_title.to_string(),
            page_message: None,
            page_message_is_toast: false,
            clear_query_param: None,
            site_id: None,
            site_full_title: None,
            site_publish_configured: false,
            links: vec![],
            nav_search_action: "/admin/search".to_string(),
            nav_search_value: String::new(),
            hide_nav: false,
        }
    }

    pub fn with_message(self, message: &impl ToString) -> Self {
        Self {
            page_message: Some(message.to_string()),
            page_message_is_toast: false,
            ..self
        }
    }
    pub fn with_hide_nav(self, hide_nav: bool) -> Self {
        Self { hide_nav, ..self }
    }

    pub fn with_toast_message(
        self,
        message: &impl ToString,
        clear_query_param: &impl ToString,
    ) -> Self {
        Self {
            page_message: Some(message.to_string()),
            page_message_is_toast: true,
            clear_query_param: Some(clear_query_param.to_string()),
            ..self
        }
    }

    pub fn with_site_context(self, site: &crate::entities::site::Model) -> Self {
        Self {
            document_title: format!("{} - {}", self.page_title, site.full_title),
            site_id: Some(site.id),
            site_full_title: Some(site.full_title.clone()),
            nav_search_action: format!("/admin/site/{}/search", site.id),
            ..self
        }
    }

    pub fn with_site_publish_configured(self, configured: bool) -> Self {
        Self {
            site_publish_configured: configured,
            ..self
        }
    }

    pub fn with_links(self, links: Vec<AdminLink>) -> Self {
        Self { links, ..self }
    }

    pub fn with_nav_search_value(self, value: &impl ToString) -> Self {
        Self {
            nav_search_value: value.to_string(),
            ..self
        }
    }
}

#[derive(Clone)]
pub(crate) struct AdminState {
    pub(crate) db: Arc<DatabaseConnection>,
    pub(crate) oidc_client_id: ClientId,
    pub(crate) oidc_client_secret: Option<ClientSecret>,
    pub(crate) oidc_frontend_url: Url,
    pub(crate) oidc_discovery_url: IssuerUrl,
    pub(crate) oidc_client: Arc<reqwest::Client>,
    pub(crate) jwt_signer: Arc<compact_jwt::JwsHs256Signer>,
    pub(crate) jwt_issuer: String,
    pub(crate) upload_root: PathBuf,
    pub(crate) log_path: PathBuf,
    pub(crate) site_templates_root: PathBuf,
    pub(crate) rendered_root: PathBuf,
}

pub(crate) async fn site_has_publish_config<C: ConnectionTrait>(
    db: &C,
    site_id: Uuid,
) -> Result<bool, SiteError> {
    Ok(crate::publish::get_site_publish_config(db, site_id)
        .await?
        .is_some_and(|config| {
            config.method != crate::entities::site_publish_config::PublishMethod::Disabled
        }))
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_index.html")]
pub(crate) struct AdminIndexTemplate {
    pub(crate) template_shared: AdminTemplateData,
    pub(crate) sites: Vec<crate::entities::site::Model>,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_themes.html")]
pub(crate) struct AdminThemesTemplate {
    pub(crate) template_shared: AdminTemplateData,
    pub(crate) themes: Vec<ThemeAdminRow>,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "not_found.html")]
pub(crate) struct NotFoundTemplate {
    pub(crate) requested_path: String,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_sites_new.html")]
pub(crate) struct AdminSitesNewTemplate {
    pub(crate) template_shared: AdminTemplateData,

    pub(crate) templates: Vec<String>,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_sites_import.html")]
pub(crate) struct AdminSitesImportTemplate {
    pub(crate) template_shared: AdminTemplateData,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_content_search.html")]
pub(crate) struct AdminSearchTemplate {
    pub(crate) template_shared: AdminTemplateData,
    pub(crate) rows: Vec<AdminSearchRow>,
    pub(crate) show_site_column: bool,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_content_detail.html")]
pub(crate) struct AdminContentDetailTemplate {
    pub(crate) template_shared: AdminTemplateData,
    pub(crate) title: String,
    pub(crate) page_type: PageType,
    pub(crate) status: String,
    pub(crate) primary_route: String,
    pub(crate) revisions_summary: String,
    pub(crate) tags: Vec<String>,
    pub(crate) aliases: Vec<AdminContentAliasRow>,
    pub(crate) creator_label: String,
    pub(crate) content_id: Uuid,
    pub(crate) site_id: Uuid,
    pub(crate) slug: String,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) published_at: String,
    pub(crate) page_content: String,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_content_revisions.html")]
pub(crate) struct AdminContentRevisionsTemplate {
    pub(crate) template_shared: AdminTemplateData,

    pub(crate) rows: Vec<AdminRow>,
    pub(crate) inline_body: String,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_revision_diff.html")]
pub(crate) struct AdminRevisionDiffTemplate {
    pub(crate) template_shared: AdminTemplateData,

    pub(crate) rows: Vec<AdminRow>,
    pub(crate) pre_body: String,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_assets.html")]
pub(crate) struct AdminAssetsTemplate {
    pub(crate) template_shared: AdminTemplateData,
    pub(crate) site_id: Uuid,
    pub(crate) site_full_title: String,
    pub(crate) sort_by_options: Vec<AdminSelectOption>,
    pub(crate) sort_dir_options: Vec<AdminSelectOption>,
    pub(crate) assets: Vec<AdminAssetRow>,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_assets_new.html")]
pub(crate) struct AdminAssetsNewTemplate {
    pub(crate) template_shared: AdminTemplateData,
    pub(crate) site_id: Uuid,
    pub(crate) site_full_title: String,
    pub(crate) sort_by_options: Vec<AdminSelectOption>,
    pub(crate) sort_dir_options: Vec<AdminSelectOption>,
    pub(crate) assets: Vec<AdminAssetRow>,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_assets_replace.html")]
pub(crate) struct AdminAssetReplaceTemplate {
    pub(crate) template_shared: AdminTemplateData,
    pub(crate) site_id: Uuid,
    pub(crate) site_full_title: String,
    pub(crate) asset: AdminAssetRow,
}
#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_render.html")]
pub(crate) struct AdminRenderTemplate {
    pub(crate) template_shared: AdminTemplateData,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_content_list.html")]
pub(crate) struct AdminContentListTemplate {
    pub(crate) template_shared: AdminTemplateData,

    pub(crate) site_id: Uuid,
    pub(crate) page_type_options: Vec<AdminSelectOption>,
    pub(crate) status_options: Vec<AdminSelectOption>,
    pub(crate) current_sort_by: &'static str,
    pub(crate) sort_headers: Vec<AdminContentListSortHeader>,
    pub(crate) content_rows: Vec<AdminContentListRow>,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_tags.html")]
pub(crate) struct AdminTagsTemplate {
    pub(crate) template_shared: AdminTemplateData,
    pub(crate) site_id: Uuid,
    pub(crate) tags: Vec<AdminSiteTagRow>,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "content_new.html")]
pub(crate) struct AdminContentNewTemplate {
    pub(crate) template_shared: AdminTemplateData,
    pub(crate) tags: Vec<AdminTagOption>,
    pub(crate) page_content: String,
    pub(crate) site_id: Uuid,
    pub(crate) allow_external_image: bool,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_content_source.html")]
pub(crate) struct AdminContentSourceTemplate {
    pub(crate) template_shared: AdminTemplateData,
    pub(crate) tags: Vec<AdminTagOption>,
    pub(crate) title: String,
    pub(crate) slug: String,
    pub(crate) page_type: String,
    pub(crate) draft: bool,
    pub(crate) published_at: String,
    pub(crate) page_content: String,
    pub(crate) site_id: Uuid,
    pub(crate) allow_external_image: bool,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_content_scan.html")]
pub(crate) struct AdminContentScanTemplate {
    pub(crate) template_shared: AdminTemplateData,
    pub(crate) site_id: Uuid,
    pub(crate) domains: String,
    pub(crate) scan_limit: usize,
    pub(crate) filter_options: Vec<AdminSelectOption>,
    pub(crate) current_filter: String,
    pub(crate) results: Vec<AdminContentScanResult>,
    pub(crate) summary: Option<AdminContentScanSummary>,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "site_settings.html")]
pub(crate) struct AdminSiteSettingsTemplate {
    pub(crate) template_shared: AdminTemplateData,

    pub(crate) site_id: Uuid,
    pub(crate) site_short_name: String,
    pub(crate) full_title: String,
    pub(crate) template_name: String,
    pub(crate) publish_on_render: bool,
    pub(crate) can_delete_site: bool,
    pub(crate) can_import_wordpress: bool,
    pub(crate) can_manage_publish: bool,
    pub(crate) templates: Vec<String>,
    pub(crate) template_files: Vec<AdminSiteTemplateFileRow>,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "site_publish.html")]
pub(crate) struct AdminSitePublishTemplate {
    pub(crate) template_shared: AdminTemplateData,
    pub(crate) site_id: Uuid,
    pub(crate) site_short_name: String,
    pub(crate) full_title: String,
    pub(crate) method_label: String,
    pub(crate) method_description: String,
    pub(crate) endpoint_url: String,
    pub(crate) bucket: String,
    pub(crate) prefix: String,
    pub(crate) region: String,
    pub(crate) access_key_id: String,
    pub(crate) secret_present: bool,
    pub(crate) force_path_style: bool,
    pub(crate) ssh_host: String,
    pub(crate) ssh_user: String,
    pub(crate) ssh_port: String,
    pub(crate) remote_path: String,
    pub(crate) identity_file: String,
    pub(crate) can_publish_now: bool,
    pub(crate) csrf_token: String,
    pub(crate) publish_csrf_token: String,
    pub(crate) runs: Vec<AdminSitePublishRunRow>,
    pub(crate) publish_methods: Vec<PublishMethod>,
    pub(crate) current_publish_method: PublishMethod,
    pub(crate) show_s3_config: bool,
    pub(crate) show_rsync_config: bool,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_logs.html")]
pub(crate) struct AdminLogsTemplate {
    pub(crate) template_shared: AdminTemplateData,
    pub(crate) log_file_path: String,
    pub(crate) search_query: String,
    pub(crate) level_filter: String,
    pub(crate) line_limit: usize,
    pub(crate) total_lines: usize,
    pub(crate) matched_lines: usize,
    pub(crate) truncated: bool,
    pub(crate) lines: Vec<String>,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "site_publish_run.html")]
pub(crate) struct AdminSitePublishRunTemplate {
    pub(crate) template_shared: AdminTemplateData,
    pub(crate) site_id: Uuid,
    pub(crate) site_short_name: String,
    pub(crate) full_title: String,
    pub(crate) run_id: Uuid,
    pub(crate) status: String,
    pub(crate) method: String,
    pub(crate) actor_sub: String,
    pub(crate) created_at: String,
    pub(crate) started_at: String,
    pub(crate) finished_at: String,
    pub(crate) rendered_file_count: i32,
    pub(crate) published_file_count: i32,
    pub(crate) deleted_object_count: i32,
    pub(crate) error_message: String,
    pub(crate) log_file_path: String,
    pub(crate) search_query: String,
    pub(crate) level_filter: String,
    pub(crate) line_limit: usize,
    pub(crate) total_lines: usize,
    pub(crate) matched_lines: usize,
    pub(crate) truncated: bool,
    pub(crate) lines: Vec<String>,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "site_delete_confirm.html")]
pub(crate) struct AdminSiteDeleteConfirmTemplate {
    pub(crate) template_shared: AdminTemplateData,

    pub(crate) site_id: Uuid,
    pub(crate) site_full_title: String,
    pub(crate) site_short_name: String,
    pub(crate) csrf_token: String,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "site_template_editor.html")]
pub(crate) struct AdminSiteTemplateEditorTemplate {
    pub(crate) template_shared: AdminTemplateData,
    pub(crate) site_id: Uuid,
    pub(crate) site_full_title: String,
    pub(crate) template_name: String,
    pub(crate) file_name: String,
    pub(crate) source: String,
    pub(crate) source_origin: String,
    pub(crate) override_exists: bool,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "memberships.html")]
pub(crate) struct AdminMembershipsTemplate {
    pub(crate) template_shared: AdminTemplateData,

    pub(crate) site_id: Uuid,
    pub(crate) site_full_title: String,
    pub(crate) roles: Vec<SiteRole>,
    pub(crate) memberships: Vec<AdminMembershipRow>,
    pub(crate) membership_candidates: Vec<AdminMembershipCandidateRow>,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_user_profile.html")]
pub(crate) struct AdminUserProfileTemplate {
    pub(crate) template_shared: AdminTemplateData,

    pub(crate) user_id: Uuid,
    pub(crate) display_name: String,
    pub(crate) subject: String,
    pub(crate) email: String,
    pub(crate) created_at: String,
    pub(crate) last_login_at: String,
    pub(crate) is_admin: bool,
    pub(crate) memberships: Vec<AdminUserMembershipRow>,
    pub(crate) token_grant_options: Vec<AdminUserTokenGrantRow>,
    pub(crate) tokens: Vec<AdminUserTokenRow>,
    pub(crate) issued_token: Option<String>,
    pub(crate) issue_token_csrf_token: String,
    pub(crate) revoke_token_csrf_token: String,
    pub(crate) can_manage_tokens: bool,
    pub(crate) can_create_users: bool,
}

#[allow(dead_code)]
#[derive(Template, WebTemplate)]
#[template(path = "admin_users.html")]
pub(crate) struct AdminUsersTemplate {
    pub(crate) template_shared: AdminTemplateData,
    pub(crate) users: Vec<AdminUserListRow>,
    pub(crate) create_user_csrf_token: String,
}

#[derive(Debug)]
pub(crate) struct AdminRow {
    pub(crate) label: String,
    pub(crate) value: String,
}

#[derive(Debug)]
pub(crate) struct AdminContentAliasRow {
    pub(crate) kind: String,
    pub(crate) path: String,
}

#[derive(Debug)]
pub(crate) struct AdminTagOption {
    pub(crate) name: String,
    pub(crate) selected: bool,
}

#[derive(Debug)]
pub(crate) struct AdminSelectOption {
    pub(crate) label: &'static str,
    pub(crate) value: &'static str,
    pub(crate) selected: bool,
}

#[derive(Debug)]
pub(crate) struct AdminSearchRow {
    pub(crate) site_title: String,
    pub(crate) edit_href: String,
    pub(crate) title: String,
    pub(crate) created_at: String,
}

#[derive(Debug)]
pub(crate) struct AdminContentListRow {
    pub(crate) edit_href: String,
    pub(crate) title: String,
    pub(crate) page_type: String,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

#[derive(Debug)]
pub(crate) struct AdminContentListSortHeader {
    pub(crate) label: &'static str,
    pub(crate) href: String,
    pub(crate) indicator: &'static str,
}

#[derive(Debug)]
pub(crate) struct AdminSiteTagRow {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) delete_href: String,
}

#[derive(Debug)]
pub(crate) struct AdminMembershipRow {
    pub(crate) subject: String,
    pub(crate) email: Option<String>,
    pub(crate) role: SiteRole,
    pub(crate) profile_href: Option<String>,
    pub(crate) update_href: String,
    pub(crate) remove_href: String,
}

#[derive(Debug)]
pub(crate) struct AdminMembershipCandidateRow {
    pub(crate) user_id: Uuid,
    pub(crate) subject: String,
    pub(crate) email: Option<String>,
    pub(crate) search_value: String,
}

#[derive(Debug)]
pub(crate) struct AdminUserMembershipRow {
    pub(crate) site_title: String,
    pub(crate) site_short_name: String,
    pub(crate) role: SiteRole,
    pub(crate) site_href: String,
}

#[derive(Debug)]
pub(crate) struct AdminUserListRow {
    pub(crate) profile_href: String,
    pub(crate) subject: String,
    pub(crate) display_name: String,
    pub(crate) email: String,
    pub(crate) is_admin: bool,
}

#[derive(Debug)]
pub(crate) struct AdminUserTokenGrantRoleOption {
    pub(crate) value: &'static str,
    pub(crate) label: String,
    pub(crate) selected: bool,
}

#[derive(Debug)]
pub(crate) struct AdminUserTokenGrantRow {
    pub(crate) site_id: Uuid,
    pub(crate) site_title: String,
    pub(crate) current_role: SiteRole,
    pub(crate) role_options: Vec<AdminUserTokenGrantRoleOption>,
}

#[derive(Debug)]
pub(crate) struct AdminUserTokenRow {
    pub(crate) label: String,
    pub(crate) grants_summary: String,
    pub(crate) created_at: String,
    pub(crate) last_used_at: String,
    pub(crate) inactive_expires_at: String,
    pub(crate) revoked_at: String,
    pub(crate) can_revoke: bool,
    pub(crate) revoke_href: String,
}

#[derive(Default)]
pub(crate) struct AdminUserProfileViewState {
    pub(crate) issued_token: Option<String>,
    pub(crate) page_message: Option<String>,
    pub(crate) page_message_is_toast: bool,
    pub(crate) clear_query_param: Option<String>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct AdminAssetRow {
    pub(crate) id: Uuid,
    pub(crate) original_filename: String,
    pub(crate) storage_basename: String,
    pub(crate) uploader_sub: String,
    pub(crate) mime_type: String,
    pub(crate) byte_length: i32,
    pub(crate) dimensions: String,
    pub(crate) created_at: String,
    pub(crate) original_url: String,
    pub(crate) thumbnail_url: Option<String>,
    pub(crate) original_exists: bool,
    pub(crate) thumbnail_exists: bool,
}

#[derive(Debug)]
pub(crate) struct AdminContentScanResult {
    pub(crate) title: String,
    pub(crate) edit_href: String,
    pub(crate) detail_href: String,
    pub(crate) issue_count: usize,
    pub(crate) fixable_count: usize,
    pub(crate) review_count: usize,
    pub(crate) issues: Vec<AdminContentScanIssueRow>,
}

#[derive(Debug)]
pub(crate) struct AdminContentScanIssueRow {
    pub(crate) issue_id: String,
    pub(crate) kind: String,
    pub(crate) label: String,
    pub(crate) current_value: String,
    pub(crate) proposed_value: String,
    pub(crate) snippet: String,
    pub(crate) can_apply: bool,
    pub(crate) selected: bool,
    pub(crate) needs_asset: bool,
    pub(crate) can_import_remote: bool,
    pub(crate) remote_url: String,
    pub(crate) selected_asset_id: String,
    pub(crate) selected_asset_label: String,
}

#[derive(Debug)]
pub(crate) struct AdminContentScanSummary {
    pub(crate) inspected_count: usize,
    pub(crate) result_count: usize,
    pub(crate) issue_count: usize,
    pub(crate) applied_count: usize,
    pub(crate) updated_items: Vec<AdminContentScanUpdatedItem>,
    pub(crate) skipped_messages: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct AdminContentScanUpdatedItem {
    pub(crate) title: String,
    pub(crate) applied_count: usize,
}

#[derive(Debug)]
pub(crate) struct AdminSiteTemplateFileRow {
    pub(crate) file_name: String,
    pub(crate) source_origin: String,
    pub(crate) edit_href: String,
    pub(crate) reset_href: String,
    pub(crate) override_exists: bool,
}

#[derive(Debug)]
pub(crate) struct AdminSitePublishRunRow {
    pub(crate) id: Uuid,
    pub(crate) detail_href: String,
    pub(crate) status: String,
    pub(crate) method: String,
    pub(crate) actor_sub: String,
    pub(crate) created_at: String,
    pub(crate) started_at: String,
    pub(crate) finished_at: String,
    pub(crate) rendered_file_count: i32,
    pub(crate) published_file_count: i32,
    pub(crate) deleted_object_count: i32,
    pub(crate) error_message: String,
}

#[derive(Debug)]
pub(crate) struct AdminLink {
    pub(crate) href: String,
    pub(crate) label: String,
    pub(crate) target: Option<String>,
}

impl AdminLink {
    pub(crate) fn new(href: &str, label: &str) -> Self {
        Self {
            href: href.to_string(),
            label: label.to_string(),
            target: None,
        }
    }
    pub(crate) fn with_target_blank(self) -> Self {
        Self {
            target: Some("_blank".to_string()),
            ..self
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateSiteForm {
    pub(crate) short_name: String,
    pub(crate) full_title: String,
    pub(crate) template_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateSiteSettingsForm {
    pub(crate) full_title: String,
    pub(crate) template_name: String,
    pub(crate) publish_on_render: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateSitePublishForm {
    pub(crate) csrf_token: String,
    pub(crate) method: String,
    pub(crate) endpoint_url: String,
    pub(crate) bucket: String,
    pub(crate) prefix: Option<String>,
    pub(crate) region: String,
    pub(crate) access_key_id: String,
    pub(crate) secret_access_key: Option<String>,
    pub(crate) force_path_style: Option<String>,
    pub(crate) ssh_host: String,
    pub(crate) ssh_user: Option<String>,
    pub(crate) ssh_port: Option<String>,
    pub(crate) remote_path: String,
    pub(crate) identity_file: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SitePublishActionForm {
    pub(crate) csrf_token: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ThemeInstallForm {
    pub(crate) repo_url: String,
    pub(crate) slug: Option<String>,
    pub(crate) branch: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub(crate) struct SiteImportLookupQuery {
    pub(crate) short_name: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CsrfTokenForm {
    pub(crate) csrf_token: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DashboardQuery {
    pub(crate) imported: Option<String>,
    pub(crate) deleted: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminUsersQuery {
    pub(crate) created: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Serialize)]
pub(crate) struct SiteImportLookupResponse {
    pub(crate) short_name: String,
    pub(crate) exists: bool,
    pub(crate) full_title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminThemesQuery {
    pub(crate) installed: Option<String>,
    pub(crate) updated: Option<String>,
    pub(crate) deleted: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminUserProfileQuery {
    pub(crate) revoked: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminContentListQuery {
    pub(crate) page_type: Option<String>,
    pub(crate) status: Option<ContentListStatusFilter>,
    pub(crate) sort_by: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminAssetListQuery {
    pub(crate) sort_by: Option<String>,
    pub(crate) sort_dir: Option<String>,
    pub(crate) uploaded: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminSiteRenderQuery {
    pub(crate) publish: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminSitePublishQuery {
    pub(crate) saved: Option<usize>,
    pub(crate) queued: Option<usize>,
    pub(crate) disabled: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminLogsQuery {
    pub(crate) q: Option<String>,
    pub(crate) level: Option<String>,
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminSitePublishRunQuery {
    pub(crate) q: Option<String>,
    pub(crate) level: Option<String>,
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateSiteTemplateOverrideForm {
    pub(crate) source: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateContentForm {
    pub(crate) page_type: String,
    pub(crate) title: String,
    pub(crate) slug: String,
    pub(crate) page_content: String,
    pub(crate) draft: Option<bool>,
    pub(crate) tag_list: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateContentForm {
    pub(crate) page_type: String,
    pub(crate) title: String,
    pub(crate) slug: String,
    pub(crate) page_content: String,
    pub(crate) draft: String,
    pub(crate) published_at: Option<String>,
    pub(crate) tag_list: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateTagForm {
    pub(crate) name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct MembershipCreateForm {
    pub(crate) subject: String,
    pub(crate) user_id: Option<Uuid>,
    pub(crate) role: SiteRole,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MembershipUpdateForm {
    pub(crate) role: SiteRole,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AdminUserCreateForm {
    pub(crate) csrf_token: String,
    pub(crate) subject: String,
    pub(crate) email: Option<String>,
    pub(crate) display_name: Option<String>,
    pub(crate) admin: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SearchQuery {
    pub(crate) q: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SourceEditorQuery {
    pub(crate) saved: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ContentScanForm {
    pub(crate) domains: String,
    pub(crate) scan_limit: Option<usize>,
    pub(crate) filter: Option<String>,
    pub(crate) content_id: Option<Uuid>,
}

#[derive(Debug)]
pub(crate) struct ContentScanApplyForm {
    pub(crate) domains: String,
    pub(crate) scan_limit: Option<usize>,
    pub(crate) filter: Option<String>,
    pub(crate) selected_issue_ids_json: Option<String>,
    pub(crate) remote_import_issue_ids_json: Option<String>,
    pub(crate) remote_import_issue_id: Vec<String>,
    pub(crate) asset_selections_json: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ManualAssetSelection {
    pub(crate) asset_id: Uuid,
    pub(crate) variant: String,
    pub(crate) asset_label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentListPageTypeFilter {
    All,
    Page,
    Post,
}

impl ContentListPageTypeFilter {
    pub(crate) fn from_query(value: Option<&str>) -> Self {
        match value {
            Some("page") => Self::Page,
            Some("post") => Self::Post,
            _ => Self::All,
        }
    }

    pub(crate) fn page_type(self) -> Option<PageType> {
        match self {
            Self::All => None,
            Self::Page => Some(PageType::Page),
            Self::Post => Some(PageType::Post),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Page => "page",
            Self::Post => "post",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "All types",
            Self::Page => "Pages",
            Self::Post => "Posts",
        }
    }

    pub(crate) fn options(self) -> Vec<AdminSelectOption> {
        [Self::All, Self::Page, Self::Post]
            .into_iter()
            .map(|option| AdminSelectOption {
                label: option.label(),
                value: option.as_str(),
                selected: option == self,
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentListStatusFilter {
    All,
    Draft,
    Published,
}

impl ContentListStatusFilter {
    pub(crate) fn draft(self) -> Option<bool> {
        match self {
            Self::All => None,
            Self::Draft => Some(true),
            Self::Published => Some(false),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Draft => "draft",
            Self::Published => "published",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "All statuses",
            Self::Draft => "Draft",
            Self::Published => "Published",
        }
    }

    pub(crate) fn options(self) -> Vec<AdminSelectOption> {
        [Self::All, Self::Draft, Self::Published]
            .into_iter()
            .map(|option| AdminSelectOption {
                label: option.label(),
                value: option.as_str(),
                selected: option == self,
            })
            .collect()
    }
}

impl<'de> Deserialize<'de> for ContentListStatusFilter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "all" => Ok(Self::All),
            "draft" => Ok(Self::Draft),
            "published" => Ok(Self::Published),
            _ => Err(serde::de::Error::unknown_variant(
                &value,
                &["all", "draft", "published"],
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentListSortBy {
    TitleAsc,
    TitleDesc,
    TypeAsc,
    TypeDesc,
    CreatedDesc,
    CreatedAsc,
    UpdatedDesc,
    UpdatedAsc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssetSortBy {
    Uploaded,
    Size,
    Name,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssetSortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentListSortColumn {
    Title,
    Type,
    Created,
    Updated,
}

impl ContentListSortBy {
    pub(crate) fn from_query(value: Option<&str>) -> Self {
        match value {
            Some("title_asc") => Self::TitleAsc,
            Some("title_desc") => Self::TitleDesc,
            Some("type_asc") => Self::TypeAsc,
            Some("type_desc") => Self::TypeDesc,
            Some("created_asc") | Some("oldest") => Self::CreatedAsc,
            Some("updated_desc") => Self::UpdatedDesc,
            Some("updated_asc") => Self::UpdatedAsc,
            Some("created_desc") | Some("newest") | None => Self::CreatedDesc,
            Some(_) => Self::CreatedDesc,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::TitleAsc => "title_asc",
            Self::TitleDesc => "title_desc",
            Self::TypeAsc => "type_asc",
            Self::TypeDesc => "type_desc",
            Self::CreatedDesc => "created_desc",
            Self::CreatedAsc => "created_asc",
            Self::UpdatedDesc => "updated_desc",
            Self::UpdatedAsc => "updated_asc",
        }
    }

    pub(crate) fn next_for_column(self, column: ContentListSortColumn) -> Self {
        match column {
            ContentListSortColumn::Title => match self {
                Self::TitleAsc => Self::TitleDesc,
                Self::TitleDesc => Self::TitleAsc,
                _ => Self::TitleAsc,
            },
            ContentListSortColumn::Type => match self {
                Self::TypeAsc => Self::TypeDesc,
                Self::TypeDesc => Self::TypeAsc,
                _ => Self::TypeAsc,
            },
            ContentListSortColumn::Created => match self {
                Self::CreatedDesc => Self::CreatedAsc,
                Self::CreatedAsc => Self::CreatedDesc,
                _ => Self::CreatedDesc,
            },
            ContentListSortColumn::Updated => match self {
                Self::UpdatedDesc => Self::UpdatedAsc,
                Self::UpdatedAsc => Self::UpdatedDesc,
                _ => Self::UpdatedDesc,
            },
        }
    }

    pub(crate) fn indicator_for(self, column: ContentListSortColumn) -> &'static str {
        match (column, self) {
            (ContentListSortColumn::Title, Self::TitleAsc)
            | (ContentListSortColumn::Type, Self::TypeAsc)
            | (ContentListSortColumn::Created, Self::CreatedAsc)
            | (ContentListSortColumn::Updated, Self::UpdatedAsc) => " (asc)",
            (ContentListSortColumn::Title, Self::TitleDesc)
            | (ContentListSortColumn::Type, Self::TypeDesc)
            | (ContentListSortColumn::Created, Self::CreatedDesc)
            | (ContentListSortColumn::Updated, Self::UpdatedDesc) => " (desc)",
            _ => "",
        }
    }
}

impl AssetSortBy {
    pub(crate) fn from_query(value: Option<&str>) -> Self {
        match value {
            Some("size") => Self::Size,
            Some("name") => Self::Name,
            Some("uploaded") | None => Self::Uploaded,
            Some(_) => Self::Uploaded,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Uploaded => "uploaded",
            Self::Size => "size",
            Self::Name => "name",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Uploaded => "Date uploaded",
            Self::Size => "File size",
            Self::Name => "File name",
        }
    }

    pub(crate) fn options(self) -> Vec<AdminSelectOption> {
        [Self::Uploaded, Self::Size, Self::Name]
            .into_iter()
            .map(|option| AdminSelectOption {
                label: option.label(),
                value: option.as_str(),
                selected: option == self,
            })
            .collect()
    }
}

impl AssetSortDirection {
    pub(crate) fn from_query(value: Option<&str>) -> Self {
        match value {
            Some("asc") => Self::Asc,
            Some("desc") | None => Self::Desc,
            Some(_) => Self::Desc,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Asc => "Ascending",
            Self::Desc => "Descending",
        }
    }

    pub(crate) fn options(self) -> Vec<AdminSelectOption> {
        [Self::Desc, Self::Asc]
            .into_iter()
            .map(|option| AdminSelectOption {
                label: option.label(),
                value: option.as_str(),
                selected: option == self,
            })
            .collect()
    }
}

pub(crate) fn sort_assets(
    assets: &mut [entities::asset::Model],
    sort_by: AssetSortBy,
    sort_dir: AssetSortDirection,
) {
    let compare_name_ci =
        |left: &entities::asset::Model, right: &entities::asset::Model| -> std::cmp::Ordering {
            left.original_filename
                .to_lowercase()
                .cmp(&right.original_filename.to_lowercase())
        };

    match sort_by {
        AssetSortBy::Uploaded => {
            assets.sort_by(|left, right| {
                let primary = match sort_dir {
                    AssetSortDirection::Asc => left.created_at.cmp(&right.created_at),
                    AssetSortDirection::Desc => right.created_at.cmp(&left.created_at),
                };
                primary
                    .then_with(|| compare_name_ci(left, right))
                    .then_with(|| right.id.cmp(&left.id))
            });
        }
        AssetSortBy::Size => {
            assets.sort_by(|left, right| {
                let primary = match sort_dir {
                    AssetSortDirection::Asc => left.byte_length.cmp(&right.byte_length),
                    AssetSortDirection::Desc => right.byte_length.cmp(&left.byte_length),
                };
                primary
                    .then_with(|| compare_name_ci(left, right))
                    .then_with(|| right.created_at.cmp(&left.created_at))
                    .then_with(|| right.id.cmp(&left.id))
            });
        }
        AssetSortBy::Name => {
            assets.sort_by(|left, right| {
                let primary = match sort_dir {
                    AssetSortDirection::Asc => compare_name_ci(left, right),
                    AssetSortDirection::Desc => compare_name_ci(right, left),
                };
                primary
                    .then_with(|| right.created_at.cmp(&left.created_at))
                    .then_with(|| right.id.cmp(&left.id))
            });
        }
    }
}

pub(crate) fn sort_content_items(
    content_items: &mut [entities::content_item::Model],
    sort_by: ContentListSortBy,
) {
    match sort_by {
        ContentListSortBy::TitleAsc => {
            content_items.sort_by(|left, right| {
                left.title
                    .to_lowercase()
                    .cmp(&right.title.to_lowercase())
                    .then_with(|| left.created_at.cmp(&right.created_at))
            });
        }
        ContentListSortBy::TitleDesc => {
            content_items.sort_by(|left, right| {
                right
                    .title
                    .to_lowercase()
                    .cmp(&left.title.to_lowercase())
                    .then_with(|| right.created_at.cmp(&left.created_at))
            });
        }
        ContentListSortBy::TypeAsc => {
            content_items.sort_by(|left, right| {
                left.page_type
                    .as_ref()
                    .cmp(right.page_type.as_ref())
                    .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
            });
        }
        ContentListSortBy::TypeDesc => {
            content_items.sort_by(|left, right| {
                right
                    .page_type
                    .as_ref()
                    .cmp(left.page_type.as_ref())
                    .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
            });
        }
        ContentListSortBy::CreatedDesc => {
            content_items.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        }
        ContentListSortBy::CreatedAsc => {
            content_items.sort_by(|left, right| left.created_at.cmp(&right.created_at));
        }
        ContentListSortBy::UpdatedDesc => {
            content_items.sort_by(
                |left, right| match (left.last_updated, right.last_updated) {
                    (Some(left_updated), Some(right_updated)) => right_updated.cmp(&left_updated),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => right.created_at.cmp(&left.created_at),
                },
            );
        }
        ContentListSortBy::UpdatedAsc => {
            content_items.sort_by(
                |left, right| match (left.last_updated, right.last_updated) {
                    (Some(left_updated), Some(right_updated)) => left_updated.cmp(&right_updated),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => left.created_at.cmp(&right.created_at),
                },
            );
        }
    }
}

pub(crate) fn content_list_href(
    site_id: Uuid,
    page_type_filter: ContentListPageTypeFilter,
    status_filter: ContentListStatusFilter,
    sort_by: ContentListSortBy,
) -> String {
    let mut href = format!("/admin/site/{site_id}/content?sort_by={}", sort_by.as_str());
    if page_type_filter != ContentListPageTypeFilter::All {
        href.push_str("&page_type=");
        href.push_str(page_type_filter.as_str());
    }
    if status_filter != ContentListStatusFilter::All {
        href.push_str("&status=");
        href.push_str(status_filter.as_str());
    }
    href
}

pub(crate) fn build_content_list_sort_headers(
    site_id: Uuid,
    page_type_filter: ContentListPageTypeFilter,
    status_filter: ContentListStatusFilter,
    sort_by: ContentListSortBy,
) -> Vec<AdminContentListSortHeader> {
    [
        (ContentListSortColumn::Title, "Title"),
        (ContentListSortColumn::Type, "Type"),
        (ContentListSortColumn::Created, "Created"),
        (ContentListSortColumn::Updated, "Updated"),
    ]
    .into_iter()
    .map(|(column, label)| AdminContentListSortHeader {
        label,
        href: content_list_href(
            site_id,
            page_type_filter,
            status_filter,
            sort_by.next_for_column(column),
        ),
        indicator: sort_by.indicator_for(column),
    })
    .collect()
}

pub(crate) fn build_search_rows(
    items: Vec<entities::content_item::Model>,
    site_title_by_id: &HashMap<Uuid, String>,
) -> Vec<AdminSearchRow> {
    items
        .into_iter()
        .map(|row| AdminSearchRow {
            site_title: site_title_by_id
                .get(&row.site_id)
                .cloned()
                .unwrap_or_else(|| "Unknown site".to_string()),
            edit_href: format!("/admin/site/{}/content/{}/edit", row.site_id, row.id),
            title: row.title,
            created_at: row.created_at.to_rfc3339(),
        })
        .collect()
}

pub(crate) fn site_delete_csrf_scope(site_id: Uuid) -> String {
    format!("delete-site:{site_id}")
}

pub(crate) fn site_publish_csrf_scope(site_id: Uuid) -> String {
    format!("site-publish:{site_id}")
}

pub(crate) fn site_publish_run_csrf_scope(site_id: Uuid) -> String {
    format!("site-publish-run:{site_id}")
}

pub(crate) fn admin_user_create_csrf_scope() -> &'static str {
    "create-user"
}

pub(crate) fn user_token_issue_csrf_scope(user_id: Uuid) -> String {
    format!("issue-user-token:{user_id}")
}

pub(crate) fn user_token_revoke_csrf_scope(user_id: Uuid) -> String {
    format!("revoke-user-token:{user_id}")
}

pub(crate) fn no_store_response(response: impl IntoResponse) -> Response {
    let mut response = response.into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store, max-age=0"),
    );
    response
        .headers_mut()
        .insert(header::PRAGMA, header::HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert(header::EXPIRES, header::HeaderValue::from_static("0"));
    response
}

pub(crate) fn parse_tag_list(raw: Option<String>) -> Vec<String> {
    raw.unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

pub(crate) fn scan_filter_value(value: Option<&str>) -> &'static str {
    match value {
        Some("fixable") => "fixable",
        Some("review") => "review",
        Some("asset") => "asset",
        _ => "all",
    }
}

pub(crate) fn content_scan_filter_options(current: &str) -> Vec<AdminSelectOption> {
    [
        ("All issues", "all"),
        ("Fixable only", "fixable"),
        ("Review only", "review"),
        ("Asset issues", "asset"),
    ]
    .into_iter()
    .map(|(label, value)| AdminSelectOption {
        label,
        value,
        selected: current == value,
    })
    .collect()
}

pub(crate) struct LoadedContentScanReports {
    pub(crate) reports: Vec<ContentScanReport>,
    pub(crate) inspected_count: usize,
}

pub(crate) fn normalize_content_scan_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(5).clamp(1, 50)
}

pub(crate) async fn load_content_scan_reports(
    db: &DatabaseConnection,
    site_id: Uuid,
    content_id: Option<Uuid>,
    domains_raw: &str,
    scan_limit: usize,
) -> Result<LoadedContentScanReports, SiteError> {
    let context = ScanContext::load(db, site_id, content_id, domains_raw).await?;
    let mut content_items = list_content(db, site_id, None, None)
        .await
        .map_err(SiteError::internal)?;
    content_items.sort_by(|left, right| {
        left.last_updated
            .unwrap_or(left.created_at)
            .cmp(&right.last_updated.unwrap_or(right.created_at))
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
    });

    let mut reports = Vec::new();
    let mut inspected_count = 0usize;
    for content in content_items {
        inspected_count = inspected_count.saturating_add(1);
        let report = scan_content(&content, &context);
        if report.issues.is_empty() {
            continue;
        }
        reports.push(report);
        if reports.len() >= scan_limit {
            break;
        }
    }

    Ok(LoadedContentScanReports {
        reports,
        inspected_count,
    })
}

pub(crate) fn build_content_scan_template(
    site: &entities::site::Model,
    site_publish_configured: bool,
    domains: String,
    scan_limit: usize,
    current_filter: &str,
    reports: Vec<ContentScanReport>,
    summary: Option<AdminContentScanSummary>,
) -> AdminContentScanTemplate {
    let results = build_content_scan_results(site.id, current_filter, reports);
    AdminContentScanTemplate {
        template_shared: AdminTemplateData::new("Content Remediation")
            .with_site_context(site)
            .with_site_publish_configured(site_publish_configured)
            .with_links(vec![
                AdminLink::new(
                    &format!("/admin/site/{}/content", site.id),
                    "Back to content",
                ),
                AdminLink::new(&format!("/admin/site/{}/assets", site.id), "Assets"),
            ]),
        site_id: site.id,
        domains,
        scan_limit,
        filter_options: content_scan_filter_options(current_filter),
        current_filter: current_filter.to_string(),
        results,
        summary,
    }
}

pub(crate) fn build_content_scan_results(
    site_id: Uuid,
    current_filter: &str,
    reports: Vec<ContentScanReport>,
) -> Vec<AdminContentScanResult> {
    reports
        .into_iter()
        .filter_map(|report| {
            let issues = report
                .issues
                .into_iter()
                .filter(|issue| issue_matches_filter(issue, current_filter))
                .map(build_scan_issue_row)
                .collect::<Vec<_>>();
            if issues.is_empty() {
                return None;
            }
            let fixable_count = issues.iter().filter(|issue| issue.can_apply).count();
            let review_count = issues.len().saturating_sub(fixable_count);
            Some(AdminContentScanResult {
                title: report.content.title,
                edit_href: format!("/admin/site/{site_id}/content/{}/edit", report.content.id),
                detail_href: format!("/admin/site/{site_id}/content/{}", report.content.id),
                issue_count: issues.len(),
                fixable_count,
                review_count,
                issues,
            })
        })
        .collect()
}

pub(crate) fn issue_matches_filter(issue: &ScanIssue, current_filter: &str) -> bool {
    match current_filter {
        "fixable" => !matches!(issue.action, ScanAction::ReviewOnly),
        "review" => matches!(issue.action, ScanAction::ReviewOnly),
        "asset" => matches!(issue.action, ScanAction::ReplaceAsset { .. }),
        _ => true,
    }
}

pub(crate) fn build_scan_issue_row(issue: ScanIssue) -> AdminContentScanIssueRow {
    let (
        can_apply,
        needs_asset,
        can_import_remote,
        remote_url,
        selected_asset_id,
        selected_asset_label,
    ) = match &issue.action {
        ScanAction::ReplaceText { .. } => (
            true,
            false,
            false,
            String::new(),
            String::new(),
            String::new(),
        ),
        ScanAction::ReplaceAsset {
            suggested_asset,
            remote_url,
            ..
        } => {
            let suggested_asset_id = suggested_asset
                .as_ref()
                .map(|asset| format!("{}:{}", asset.asset_id, asset.variant))
                .unwrap_or_default();
            let suggested_asset_label = suggested_asset
                .as_ref()
                .map(|asset| format!("{} ({})", asset.asset_label, asset.variant))
                .unwrap_or_default();
            (
                true,
                suggested_asset.is_none(),
                remote_url.is_some(),
                remote_url.clone().unwrap_or_default(),
                suggested_asset_id,
                suggested_asset_label,
            )
        }
        ScanAction::ReviewOnly => (
            false,
            false,
            false,
            String::new(),
            String::new(),
            String::new(),
        ),
    };

    AdminContentScanIssueRow {
        issue_id: issue.issue_id,
        kind: issue.kind,
        label: issue.label,
        current_value: issue.current_value,
        proposed_value: issue
            .proposed_value
            .unwrap_or_else(|| "Manual selection required".to_string()),
        snippet: issue.snippet,
        can_apply,
        selected: can_apply && !needs_asset,
        needs_asset,
        can_import_remote,
        remote_url,
        selected_asset_id,
        selected_asset_label,
    }
}

pub(crate) fn build_scan_summary(
    scan_reports: &LoadedContentScanReports,
    applied_count: usize,
    updated_items: Vec<AdminContentScanUpdatedItem>,
    skipped_messages: Vec<String>,
) -> AdminContentScanSummary {
    AdminContentScanSummary {
        inspected_count: scan_reports.inspected_count,
        result_count: scan_reports.reports.len(),
        issue_count: scan_reports
            .reports
            .iter()
            .map(|report| report.issues.len())
            .sum(),
        applied_count,
        updated_items,
        skipped_messages,
    }
}

pub(crate) fn deserialize_manual_asset_map(
    raw: Option<&str>,
) -> Result<HashMap<String, ManualAssetSelection>, SiteError> {
    let raw = raw.unwrap_or("").trim();
    if raw.is_empty() {
        return Ok(HashMap::new());
    }
    serde_json::from_str(raw).map_err(|error| {
        SiteError::BadRequest(format!("invalid asset selections payload: {error}"))
    })
}

pub(crate) fn deserialize_string_set(raw: Option<&str>) -> Result<HashSet<String>, SiteError> {
    let raw = raw.unwrap_or("").trim();
    if raw.is_empty() {
        return Ok(HashSet::new());
    }
    let values = serde_json::from_str::<Vec<String>>(raw)
        .map_err(|error| SiteError::BadRequest(format!("invalid string list payload: {error}")))?;
    Ok(values.into_iter().collect())
}

#[derive(
    EnumIter,
    DeriveActiveEnum,
    Copy,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Serialize,
    Deserialize,
    Hash,
)]
#[sea_orm(
    rs_type = "String",
    db_type = "String(StringLen::None)",
    rename_all = "lowercase"
)]
#[serde(rename_all = "lowercase")]
pub enum SiteRole {
    Viewer,
    Author,
    Editor,
    Owner,
    Admin,
}

impl SiteRole {
    pub fn is_viewer(self) -> bool {
        self == Self::Viewer
    }

    pub fn is_owner(self) -> bool {
        self == Self::Owner
    }

    pub fn is_author(self) -> bool {
        self == Self::Author
    }

    pub fn is_editor(self) -> bool {
        self == Self::Editor
    }

    /// Are they a system admin?
    pub fn is_admin(self) -> bool {
        self == Self::Admin
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Author => "author",
            Self::Editor => "editor",
            Self::Owner => "owner",
            Self::Admin => "admin",
        }
    }

    pub fn all_without_admin() -> Vec<Self> {
        vec![Self::Viewer, Self::Author, Self::Editor, Self::Owner]
    }
}

impl std::fmt::Display for SiteRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SiteRole::Viewer => write!(f, "Viewer"),
            SiteRole::Author => write!(f, "Author"),
            SiteRole::Editor => write!(f, "Editor"),
            SiteRole::Owner => write!(f, "Owner"),
            SiteRole::Admin => write!(f, "Admin"),
        }
    }
}

impl std::str::FromStr for SiteRole {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "viewer" => Ok(Self::Viewer),
            "author" => Ok(Self::Author),
            "editor" => Ok(Self::Editor),
            "owner" => Ok(Self::Owner),
            "admin" => Ok(Self::Admin),
            _ => Err(format!("unsupported site role: {value}")),
        }
    }
}

pub(crate) async fn current_user(session: &Session) -> Result<entities::user::Model, SiteError> {
    session
        .get::<entities::user::Model>(SESSION_USER)
        .await
        .map_err(|_| SiteError::internal("failed to read session"))?
        .ok_or_else(|| SiteError::UnAuthorized("missing user session".to_string()))
}

pub(crate) async fn require_global_admin(
    session: &Session,
) -> Result<entities::user::Model, SiteError> {
    let user = current_user(session).await?;
    if user.admin {
        Ok(user)
    } else {
        Err(SiteError::UnAuthorized(
            "global admin access is required".to_string(),
        ))
    }
}

pub(crate) fn role_satisfies(actual: SiteRole, required: SiteRole) -> bool {
    let rank = |role: SiteRole| match role {
        SiteRole::Viewer => 0_u8,
        SiteRole::Author => 1,
        SiteRole::Editor => 2,
        SiteRole::Owner => 3,
        SiteRole::Admin => 4,
    };

    rank(actual) >= rank(required)
}

pub(crate) fn can_view_user_profile(
    viewer: &entities::user::Model,
    target: &entities::user::Model,
) -> bool {
    viewer.admin || viewer.id == target.id
}

pub(crate) fn map_transaction_error<T>(
    result: Result<T, sea_orm::TransactionError<SiteError>>,
) -> Result<T, SiteError> {
    result.map_err(|error| match error {
        sea_orm::TransactionError::Connection(error) => SiteError::from(error),
        sea_orm::TransactionError::Transaction(error) => error,
    })
}

pub(crate) fn collect_form_values(raw: &[u8]) -> HashMap<String, Vec<String>> {
    let mut values = HashMap::new();
    for (key, value) in url::form_urlencoded::parse(raw) {
        values
            .entry(key.into_owned())
            .or_insert_with(Vec::new)
            .push(value.into_owned());
    }
    values
}

pub(crate) fn first_form_value(values: &HashMap<String, Vec<String>>, key: &str) -> Option<String> {
    values.get(key).and_then(|items| items.first().cloned())
}

pub(crate) fn parse_optional_usize(
    value: Option<String>,
    field_name: &str,
) -> Result<Option<usize>, SiteError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .parse::<usize>()
        .map(Some)
        .map_err(|error| SiteError::BadRequest(format!("invalid {field_name} value: {error}")))
}

pub(crate) fn parse_content_scan_apply_form(raw: &[u8]) -> Result<ContentScanApplyForm, SiteError> {
    let values = collect_form_values(raw);
    let domains = first_form_value(&values, "domains")
        .ok_or_else(|| SiteError::BadRequest("missing domains field".to_string()))?;
    Ok(ContentScanApplyForm {
        domains,
        scan_limit: parse_optional_usize(first_form_value(&values, "scan_limit"), "scan_limit")?,
        filter: first_form_value(&values, "filter"),
        selected_issue_ids_json: first_form_value(&values, "selected_issue_ids_json"),
        remote_import_issue_ids_json: first_form_value(&values, "remote_import_issue_ids_json"),
        remote_import_issue_id: values
            .get("remote_import_issue_id")
            .cloned()
            .unwrap_or_default(),
        asset_selections_json: first_form_value(&values, "asset_selections_json"),
    })
}

pub(crate) fn role_options_up_to(max_role: SiteRole) -> Vec<AdminUserTokenGrantRoleOption> {
    SiteRole::all_without_admin()
        .into_iter()
        .filter(|role| role_satisfies(max_role, *role))
        .map(|role| AdminUserTokenGrantRoleOption {
            value: role.label(),
            label: role.to_string(),
            selected: role == max_role,
        })
        .collect()
}

pub(crate) async fn build_admin_user_profile_template(
    state: &AdminState,
    session: &Session,
    viewer: &entities::user::Model,
    target: entities::user::Model,
    view_state: AdminUserProfileViewState,
) -> Result<AdminUserProfileTemplate, SiteError> {
    let memberships = list_memberships_for_user_id(state.db.as_ref(), target.id)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load memberships: {error}")))?;
    let role_by_site = memberships
        .iter()
        .map(|membership| (membership.site_id, membership.role))
        .collect::<HashMap<_, _>>();
    let sites = list_sites_for_subject(state.db.as_ref(), &target.subject)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load sites: {error}")))?;
    let membership_rows = sites
        .iter()
        .filter_map(|site| {
            role_by_site
                .get(&site.id)
                .copied()
                .map(|role| AdminUserMembershipRow {
                    site_title: site.full_title.clone(),
                    site_short_name: site.short_name.clone(),
                    role,
                    site_href: format!("/admin/site/{}/content", site.id),
                })
        })
        .collect::<Vec<_>>();
    let token_grant_options = sites
        .iter()
        .filter_map(|site| {
            role_by_site
                .get(&site.id)
                .copied()
                .map(|role| AdminUserTokenGrantRow {
                    site_id: site.id,
                    site_title: site.full_title.clone(),
                    current_role: role,
                    role_options: role_options_up_to(role),
                })
        })
        .collect::<Vec<_>>();
    let tokens = token_auth::list_user_api_tokens(state.db.as_ref(), target.id)
        .await?
        .into_iter()
        .map(|token| {
            let grants = deserialize_grants_json(token.grants_json.as_ref())?;
            Ok(AdminUserTokenRow {
                label: token.label,
                grants_summary: summarize_grants(grants.as_ref()),
                created_at: token.created_at.to_rfc3339(),
                last_used_at: format_optional_datetime(token.last_used_at),
                inactive_expires_at: token.inactive_expires_at.to_rfc3339(),
                revoked_at: format_optional_datetime(token.revoked_at),
                can_revoke: token.revoked_at.is_none(),
                revoke_href: format!("/admin/users/{}/tokens/{}/revoke", target.id, token.id),
            })
        })
        .collect::<Result<Vec<_>, SiteError>>()?;

    let display_name = target
        .display_name
        .clone()
        .unwrap_or_else(|| "n/a".to_string());
    let profile_name = target
        .display_name
        .as_deref()
        .or(target.email.as_deref())
        .unwrap_or(&target.subject)
        .to_string();
    let template_shared = AdminTemplateData::new(&format!("User Profile: {profile_name}"));
    let template_shared = if let Some(message) = view_state.page_message {
        if view_state.page_message_is_toast {
            template_shared.with_toast_message(
                &message,
                &view_state.clear_query_param.as_deref().unwrap_or("message"),
            )
        } else {
            template_shared.with_message(&message)
        }
    } else {
        template_shared
    };
    let can_manage_tokens = can_view_user_profile(viewer, &target);
    let issue_token_csrf_token = if can_manage_tokens {
        session
            .issue_csrf_token(&user_token_issue_csrf_scope(target.id))
            .await?
    } else {
        String::new()
    };
    let revoke_token_csrf_token = if can_manage_tokens {
        session
            .issue_csrf_token(&user_token_revoke_csrf_scope(target.id))
            .await?
    } else {
        String::new()
    };

    Ok(AdminUserProfileTemplate {
        template_shared,
        user_id: target.id,
        display_name,
        subject: target.subject.clone(),
        email: target.email.clone().unwrap_or_else(|| "n/a".to_string()),
        created_at: target.created_at.to_rfc3339(),
        last_login_at: format_optional_datetime(target.last_login_at),
        is_admin: target.admin,
        memberships: membership_rows,
        token_grant_options,
        tokens,
        issued_token: view_state.issued_token,
        issue_token_csrf_token,
        revoke_token_csrf_token,
        can_manage_tokens,
        can_create_users: viewer.admin,
    })
}

pub(crate) async fn require_site_role(
    state: &AdminState,
    session: &Session,
    site_id: Uuid,
    required: SiteRole,
) -> Result<(), SiteError> {
    let user = current_user(session).await?;
    if user.admin {
        return Ok(());
    }

    let membership = get_membership_for_subject(state.db.as_ref(), site_id, &user.subject)
        .await
        .map_err(|error| SiteError::internal(format!("failed to load membership: {error}")))?;

    let Some(membership) = membership else {
        return Err(SiteError::UnAuthorized(format!(
            "missing membership for site {site_id}"
        )));
    };

    if role_satisfies(membership.role, required) {
        Ok(())
    } else {
        Err(SiteError::UnAuthorized(format!(
            "site role {} does not satisfy required role {}",
            membership.role.label(),
            required.label()
        )))
    }
}
