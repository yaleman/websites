use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use regex::{Captures, Regex};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use tracing::warn;
use url::Url;
use uuid::Uuid;

use crate::entities;
use crate::{SiteError, content_primary_route};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetReference {
    pub asset_id: Uuid,
    pub variant: String,
    pub asset_label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanAction {
    ReplaceText {
        replacement: String,
    },
    ReplaceAsset {
        alt: String,
        title: Option<String>,
        suggested_asset: Option<AssetReference>,
        remote_url: Option<String>,
    },
    ReviewOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanIssue {
    pub issue_id: String,
    pub kind: String,
    pub label: String,
    pub start: usize,
    pub end: usize,
    pub snippet: String,
    pub current_value: String,
    pub proposed_value: Option<String>,
    pub action: ScanAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentScanReport {
    pub content: entities::content_item::Model,
    pub issues: Vec<ScanIssue>,
}

#[derive(Clone, Debug)]
pub struct AssetRenderCandidate {
    pub filename: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

#[derive(Clone, Debug)]
struct LinkTarget {
    title: String,
    href: String,
}

#[derive(Clone, Debug)]
struct AssetLookup {
    asset_id: Uuid,
    asset_label: String,
    variant: String,
}

#[derive(Clone, Debug)]
pub struct ScanContext {
    internal_domains: HashSet<String>,
    link_targets: HashMap<String, LinkTarget>,
    asset_matches: HashMap<String, Vec<AssetLookup>>,
}

impl ScanContext {
    pub async fn load(
        db: &DatabaseConnection,
        site_id: Uuid,
        content_id: Option<Uuid>,
        domains: &[String],
    ) -> Result<Self, SiteError> {
        let internal_domains = domains.iter().cloned().collect::<HashSet<_>>();

        let mut content_items = entities::content_item::Entity::find()
            .filter(entities::content_item::Column::SiteId.eq(site_id));
        if let Some(page_id) = content_id {
            content_items = content_items.filter(entities::content_item::Column::Id.eq(page_id));
        }

        let content_items = content_items.all(db).await?;
        let aliases = entities::content_alias::Entity::find()
            .filter(entities::content_alias::Column::SiteId.eq(site_id))
            .all(db)
            .await?;
        let assets = entities::asset::Entity::find()
            .filter(entities::asset::Column::SiteId.eq(site_id))
            .all(db)
            .await?;
        let asset_ids = assets.iter().map(|asset| asset.id).collect::<Vec<_>>();
        let variants = if asset_ids.is_empty() {
            Vec::new()
        } else {
            entities::asset_variant::Entity::find()
                .filter(entities::asset_variant::Column::AssetId.is_in(asset_ids))
                .all(db)
                .await?
        };

        let mut content_by_id = HashMap::new();
        for content in &content_items {
            content_by_id.insert(content.id, content.clone());
        }

        let mut link_targets = HashMap::new();
        for alias in aliases {
            if let Some(content) = content_by_id.get(&alias.content_id) {
                let normalized = normalize_internal_path(&alias.alias_path);
                link_targets
                    .entry(normalized)
                    .or_insert_with(|| LinkTarget {
                        title: content.title.clone(),
                        href: display_internal_href(&alias.alias_path),
                    });
            }
        }
        for content in &content_items {
            let route = format!("/{}", content_primary_route(content).trim_matches('/'));
            let normalized = normalize_internal_path(&route);
            link_targets
                .entry(normalized)
                .or_insert_with(|| LinkTarget {
                    title: content.title.clone(),
                    href: display_internal_href(&route),
                });
        }

        let mut asset_matches: HashMap<String, Vec<AssetLookup>> = HashMap::new();
        let mut assets_by_id = HashMap::new();
        for asset in assets {
            assets_by_id.insert(asset.id, asset);
        }
        for variant in variants {
            if let Some(asset) = assets_by_id.get(&variant.asset_id) {
                add_asset_lookup(
                    &mut asset_matches,
                    &variant.filename,
                    AssetLookup {
                        asset_id: asset.id,
                        asset_label: asset.original_filename.clone(),
                        variant: variant.variant_kind.clone(),
                    },
                );
            }
        }
        for asset in assets_by_id.into_values() {
            add_asset_lookup(
                &mut asset_matches,
                &asset.storage_basename,
                AssetLookup {
                    asset_id: asset.id,
                    asset_label: asset.original_filename.clone(),
                    variant: "original".to_string(),
                },
            );
            add_asset_lookup(
                &mut asset_matches,
                &asset.original_filename,
                AssetLookup {
                    asset_id: asset.id,
                    asset_label: asset.original_filename.clone(),
                    variant: "original".to_string(),
                },
            );
        }

        Ok(Self {
            internal_domains,
            link_targets,
            asset_matches,
        })
    }

    fn find_link_target(&self, href: &str) -> Option<LinkTarget> {
        let parsed = normalize_scan_href(href, &self.internal_domains)?;
        let target = self.link_targets.get(&parsed.normalized_path)?;
        let rewritten = append_fragment(&target.href, parsed.fragment.as_deref());
        Some(LinkTarget {
            title: target.title.clone(),
            href: rewritten,
        })
    }

    fn match_asset_url(&self, href: &str) -> Option<Result<AssetReference, Option<String>>> {
        let path = asset_lookup_key(href)?;
        let matches = self.asset_matches.get(&path).cloned().unwrap_or_default();
        let Some(unique) = unique_asset_match(&matches) else {
            return Some(Err(remote_image_url(href)));
        };
        Some(Ok(AssetReference {
            asset_id: unique.asset_id,
            variant: unique.variant,
            asset_label: unique.asset_label,
        }))
    }
}

struct NormalizedHref {
    normalized_path: String,
    fragment: Option<String>,
}

pub fn parse_domain_list(raw: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    raw.split([',', '\n', '\r'])
        .filter_map(|value| {
            let trimmed = value.trim().trim_start_matches('.');
            if trimmed.is_empty() {
                return None;
            }
            let lowered = trimmed.to_ascii_lowercase();
            if seen.insert(lowered.clone()) {
                Some(lowered)
            } else {
                None
            }
        })
        .collect()
}

pub fn scan_content(
    content: &entities::content_item::Model,
    context: &ScanContext,
) -> ContentScanReport {
    let source = content.page_content.as_str();
    let mut issues = Vec::new();
    let mut occupied = Vec::new();

    scan_markdown_links_and_images(source, context, &mut issues, &mut occupied);
    scan_html_links(source, context, &mut issues, &mut occupied);
    scan_html_images(source, context, &mut issues, &mut occupied);
    scan_inline_tags(source, &mut issues, &mut occupied);
    scan_review_markup(source, &mut issues, &occupied);
    scan_bare_urls(source, context, &mut issues, &occupied);

    issues.sort_by_key(|issue| issue.start);

    ContentScanReport {
        content: content.clone(),
        issues,
    }
}

pub fn apply_issue_replacements(
    source: &str,
    issues: &[ScanIssue],
    selected_issue_ids: &HashSet<String>,
    manual_assets: &HashMap<String, AssetReference>,
    remote_imports: &HashSet<String>,
) -> Vec<AppliedIssue> {
    let mut applied = Vec::new();
    for issue in issues {
        if !selected_issue_ids.contains(&issue.issue_id) {
            continue;
        }
        match &issue.action {
            ScanAction::ReplaceText { replacement } => applied.push(AppliedIssue {
                issue_id: issue.issue_id.clone(),
                start: issue.start,
                end: issue.end,
                replacement: replacement.clone(),
                kind: issue.kind.clone(),
            }),
            ScanAction::ReplaceAsset {
                alt,
                title,
                suggested_asset,
                remote_url,
            } => {
                let asset = manual_assets
                    .get(&issue.issue_id)
                    .cloned()
                    .or_else(|| suggested_asset.clone());
                if let Some(asset) = asset {
                    applied.push(AppliedIssue {
                        issue_id: issue.issue_id.clone(),
                        start: issue.start,
                        end: issue.end,
                        replacement: format_asset_shortcode(
                            asset.asset_id,
                            asset.variant.as_str(),
                            alt,
                            title.as_deref(),
                        ),
                        kind: issue.kind.clone(),
                    });
                    continue;
                }
                if remote_imports.contains(&issue.issue_id)
                    && let Some(remote_url) = remote_url
                {
                    applied.push(AppliedIssue {
                        issue_id: issue.issue_id.clone(),
                        start: issue.start,
                        end: issue.end,
                        replacement: remote_url.clone(),
                        kind: "__remote_import__".to_string(),
                    });
                }
            }
            ScanAction::ReviewOnly => {}
        }
    }

    let _ = source;
    applied.sort_by_key(|right| std::cmp::Reverse(right.start));
    applied
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedIssue {
    pub issue_id: String,
    pub start: usize,
    pub end: usize,
    pub replacement: String,
    pub kind: String,
}

pub async fn expand_asset_shortcodes(
    db: &DatabaseConnection,
    site_id: Uuid,
    source: &str,
) -> Result<String, SiteError> {
    let mut asset_ids = HashSet::new();
    for captures in asset_shortcode_regex().captures_iter(source) {
        let Some(asset_id) = captures
            .get(1)
            .and_then(|value| Uuid::parse_str(value.as_str()).ok())
        else {
            continue;
        };
        asset_ids.insert(asset_id);
    }
    if asset_ids.is_empty() {
        return Ok(source.to_string());
    }

    let assets = entities::asset::Entity::find()
        .filter(entities::asset::Column::SiteId.eq(site_id))
        .filter(entities::asset::Column::Id.is_in(asset_ids.iter().copied().collect::<Vec<_>>()))
        .all(db)
        .await?;
    let variants = entities::asset_variant::Entity::find()
        .filter(
            entities::asset_variant::Column::AssetId
                .is_in(asset_ids.iter().copied().collect::<Vec<_>>()),
        )
        .all(db)
        .await?;

    let mut assets_by_id = HashMap::new();
    for asset in assets {
        assets_by_id.insert(asset.id, asset);
    }
    let mut variants_by_asset: HashMap<Uuid, Vec<entities::asset_variant::Model>> = HashMap::new();
    for variant in variants {
        variants_by_asset
            .entry(variant.asset_id)
            .or_default()
            .push(variant);
    }

    let rendered = asset_shortcode_regex().replace_all(source, |captures: &Captures<'_>| {
        let Some(asset_id) = captures
            .get(1)
            .and_then(|value| Uuid::parse_str(value.as_str()).ok())
        else {
            return captures
                .get(0)
                .map(|value| value.as_str().to_string())
                .unwrap_or_default();
        };
        let variant = captures
            .get(2)
            .map(|value| value.as_str())
            .unwrap_or("original");
        let alt = captures.get(3).map(|value| value.as_str()).unwrap_or("");
        let title = captures
            .get(4)
            .map(|value| value.as_str().trim())
            .unwrap_or("");

        let Some(asset) = assets_by_id.get(&asset_id) else {
            warn!(asset_id=%asset_id, "asset shortcode references missing asset");
            return captures
                .get(0)
                .map(|value| value.as_str().to_string())
                .unwrap_or_default();
        };
        let candidate = resolve_asset_render_candidate(
            asset,
            variants_by_asset
                .get(&asset_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            variant,
        );
        let Some(candidate) = candidate else {
            warn!(asset_id=%asset_id, variant, "asset shortcode references missing variant");
            return captures
                .get(0)
                .map(|value| value.as_str().to_string())
                .unwrap_or_default();
        };

        render_asset_figure_html(
            &candidate,
            alt,
            if title.is_empty() { None } else { Some(title) },
        )
    });

    Ok(rendered.into_owned())
}

pub fn format_asset_shortcode(
    asset_id: Uuid,
    variant: &str,
    alt: &str,
    title: Option<&str>,
) -> String {
    let mut shortcode = format!(
        "[[asset id=\"{}\" variant=\"{}\" alt=\"{}\"",
        asset_id,
        escape_shortcode_attr(variant),
        escape_shortcode_attr(alt)
    );
    if let Some(title) = title
        && !title.trim().is_empty()
    {
        shortcode.push_str(&format!(" title=\"{}\"", escape_shortcode_attr(title)));
    }
    shortcode.push_str("]]");
    shortcode
}

fn scan_markdown_links_and_images(
    source: &str,
    context: &ScanContext,
    issues: &mut Vec<ScanIssue>,
    occupied: &mut Vec<(usize, usize)>,
) {
    for captures in markdown_link_regex().captures_iter(source) {
        let Some(full) = captures.get(0) else {
            continue;
        };
        if overlaps(occupied, full.start(), full.end()) {
            continue;
        }
        let bang = captures.get(1).map(|value| value.as_str()).unwrap_or("");
        let label = captures
            .get(2)
            .map(|value| value.as_str())
            .unwrap_or("")
            .trim();
        let href = captures
            .get(3)
            .map(|value| value.as_str())
            .unwrap_or("")
            .trim();
        if bang == "!" {
            let asset_match = context.match_asset_url(href);
            let Some(asset_match) = asset_match else {
                continue;
            };
            let (suggested_asset, remote_url, proposed_value) = match asset_match {
                Ok(asset) => {
                    let shortcode =
                        format_asset_shortcode(asset.asset_id, &asset.variant, label, None);
                    (Some(asset), None, Some(shortcode))
                }
                Err(remote_url) => (None, remote_url.clone(), None),
            };
            issues.push(ScanIssue {
                issue_id: issue_id("markdown_image", full.start(), full.end()),
                kind: "markdown_image".to_string(),
                label: "Markdown image".to_string(),
                start: full.start(),
                end: full.end(),
                snippet: clip_snippet(source, full.start(), full.end()),
                current_value: full.as_str().to_string(),
                proposed_value,
                action: ScanAction::ReplaceAsset {
                    alt: label.to_string(),
                    title: None,
                    suggested_asset,
                    remote_url,
                },
            });
            occupied.push((full.start(), full.end()));
            continue;
        }

        let Some(target) = context.find_link_target(href) else {
            continue;
        };
        let replacement = format!("[{}]({})", escape_markdown_label(label), target.href);
        issues.push(ScanIssue {
            issue_id: issue_id("markdown_link", full.start(), full.end()),
            kind: "markdown_link".to_string(),
            label: "Markdown internal link".to_string(),
            start: full.start(),
            end: full.end(),
            snippet: clip_snippet(source, full.start(), full.end()),
            current_value: full.as_str().to_string(),
            proposed_value: Some(replacement.clone()),
            action: ScanAction::ReplaceText { replacement },
        });
        occupied.push((full.start(), full.end()));
    }
}

fn scan_html_links(
    source: &str,
    context: &ScanContext,
    issues: &mut Vec<ScanIssue>,
    occupied: &mut Vec<(usize, usize)>,
) {
    for captures in html_anchor_regex().captures_iter(source) {
        let Some(full) = captures.get(0) else {
            continue;
        };
        if overlaps(occupied, full.start(), full.end()) {
            continue;
        }
        let href = captures
            .get(1)
            .map(|value| value.as_str())
            .unwrap_or("")
            .trim();
        let text = captures
            .get(2)
            .map(|value| value.as_str())
            .unwrap_or("")
            .trim();
        if text.contains('<') {
            continue;
        }
        let replacement_href = context
            .find_link_target(href)
            .map(|target| target.href)
            .unwrap_or_else(|| href.to_string());
        let replacement = format!("[{}]({})", escape_markdown_label(text), replacement_href);
        issues.push(ScanIssue {
            issue_id: issue_id("html_link", full.start(), full.end()),
            kind: "html_link".to_string(),
            label: "HTML link".to_string(),
            start: full.start(),
            end: full.end(),
            snippet: clip_snippet(source, full.start(), full.end()),
            current_value: full.as_str().to_string(),
            proposed_value: Some(replacement.clone()),
            action: ScanAction::ReplaceText { replacement },
        });
        occupied.push((full.start(), full.end()));
    }
}

fn scan_html_images(
    source: &str,
    context: &ScanContext,
    issues: &mut Vec<ScanIssue>,
    occupied: &mut Vec<(usize, usize)>,
) {
    for captures in html_img_regex().captures_iter(source) {
        let Some(full) = captures.get(0) else {
            continue;
        };
        if overlaps(occupied, full.start(), full.end()) {
            continue;
        }
        let attributes = captures.get(1).map(|value| value.as_str()).unwrap_or("");
        let Some(src) = extract_html_attribute(attributes, "src") else {
            continue;
        };
        let alt = extract_html_attribute(attributes, "alt").unwrap_or_default();
        let title = extract_html_attribute(attributes, "title");
        let Some(asset_match) = context.match_asset_url(&src) else {
            continue;
        };
        let (suggested_asset, remote_url, proposed_value) = match asset_match {
            Ok(asset) => {
                let shortcode =
                    format_asset_shortcode(asset.asset_id, &asset.variant, &alt, title.as_deref());
                (Some(asset), None, Some(shortcode))
            }
            Err(remote_url) => (None, remote_url.clone(), None),
        };
        issues.push(ScanIssue {
            issue_id: issue_id("html_image", full.start(), full.end()),
            kind: "html_image".to_string(),
            label: "HTML image".to_string(),
            start: full.start(),
            end: full.end(),
            snippet: clip_snippet(source, full.start(), full.end()),
            current_value: full.as_str().to_string(),
            proposed_value,
            action: ScanAction::ReplaceAsset {
                alt,
                title,
                suggested_asset,
                remote_url,
            },
        });
        occupied.push((full.start(), full.end()));
    }
}

fn scan_inline_tags(source: &str, issues: &mut Vec<ScanIssue>, occupied: &mut Vec<(usize, usize)>) {
    add_simple_tag_issues(
        "p",
        "paragraph",
        |inner| format!("{}\n\n", inner.trim()),
        source,
        issues,
        occupied,
    );
    add_simple_tag_issues(
        "strong",
        "strong",
        |inner| format!("**{}**", inner.trim()),
        source,
        issues,
        occupied,
    );
    add_simple_tag_issues(
        "b",
        "bold",
        |inner| format!("**{}**", inner.trim()),
        source,
        issues,
        occupied,
    );
    add_simple_tag_issues(
        "i",
        "italic",
        |inner| format!("*{}*", inner.trim()),
        source,
        issues,
        occupied,
    );
}

fn scan_review_markup(source: &str, issues: &mut Vec<ScanIssue>, occupied: &[(usize, usize)]) {
    scan_review_regex(
        source,
        style_attr_regex(),
        "inline_style",
        "Inline style",
        issues,
        occupied,
    );
    scan_review_regex(
        source,
        class_attr_regex(),
        "inline_class",
        "Class attribute",
        issues,
        occupied,
    );
    scan_review_regex(
        source,
        dangerous_html_regex(),
        "dangerous_html",
        "Dangerous HTML",
        issues,
        occupied,
    );
    scan_review_regex(
        source,
        complex_html_regex(),
        "complex_html",
        "Complex HTML",
        issues,
        occupied,
    );
}

fn scan_bare_urls(
    source: &str,
    context: &ScanContext,
    issues: &mut Vec<ScanIssue>,
    occupied: &[(usize, usize)],
) {
    for captures in bare_url_regex().captures_iter(source) {
        let Some(url_match) = captures.get(0) else {
            continue;
        };
        if overlaps(occupied, url_match.start(), url_match.end()) {
            continue;
        }
        if inside_html_tag(source, url_match.start()) {
            continue;
        }
        let url = url_match.as_str().trim_end_matches([',', '.', ';', ')']);
        let end = url_match.start() + url.len();
        let replacement = if let Some(target) = context.find_link_target(url) {
            format!(
                "[{}]({})",
                escape_markdown_label(&target.title),
                target.href
            )
        } else {
            format!("<{}>", url)
        };
        issues.push(ScanIssue {
            issue_id: issue_id("bare_url", url_match.start(), end),
            kind: "bare_url".to_string(),
            label: "Plain URL".to_string(),
            start: url_match.start(),
            end,
            snippet: clip_snippet(source, url_match.start(), end),
            current_value: url.to_string(),
            proposed_value: Some(replacement.clone()),
            action: ScanAction::ReplaceText { replacement },
        });
    }
}

fn add_simple_tag_issues<F>(
    tag: &str,
    label: &str,
    mapper: F,
    source: &str,
    issues: &mut Vec<ScanIssue>,
    occupied: &mut Vec<(usize, usize)>,
) where
    F: Fn(&str) -> String,
{
    let regex = simple_tag_regex(tag);
    for captures in regex.captures_iter(source) {
        let Some(full) = captures.get(0) else {
            continue;
        };
        if overlaps(occupied, full.start(), full.end()) {
            continue;
        }
        let inner = captures.get(1).map(|value| value.as_str()).unwrap_or("");
        if inner.contains('<') {
            continue;
        }
        let replacement = mapper(inner);
        issues.push(ScanIssue {
            issue_id: issue_id(label, full.start(), full.end()),
            kind: label.to_string(),
            label: format!("<{}> tag", tag),
            start: full.start(),
            end: full.end(),
            snippet: clip_snippet(source, full.start(), full.end()),
            current_value: full.as_str().to_string(),
            proposed_value: Some(replacement.clone()),
            action: ScanAction::ReplaceText { replacement },
        });
        occupied.push((full.start(), full.end()));
    }
}

fn scan_review_regex(
    source: &str,
    regex: &Regex,
    kind: &str,
    label: &str,
    issues: &mut Vec<ScanIssue>,
    occupied: &[(usize, usize)],
) {
    for captures in regex.captures_iter(source) {
        let Some(full) = captures.get(0) else {
            continue;
        };
        if overlaps(occupied, full.start(), full.end()) {
            continue;
        }
        issues.push(ScanIssue {
            issue_id: issue_id(kind, full.start(), full.end()),
            kind: kind.to_string(),
            label: label.to_string(),
            start: full.start(),
            end: full.end(),
            snippet: clip_snippet(source, full.start(), full.end()),
            current_value: full.as_str().to_string(),
            proposed_value: None,
            action: ScanAction::ReviewOnly,
        });
    }
}

fn append_fragment(base: &str, fragment: Option<&str>) -> String {
    match fragment {
        Some(fragment) if !fragment.is_empty() => format!("{base}#{fragment}"),
        _ => base.to_string(),
    }
}

fn normalize_scan_href(href: &str, domains: &HashSet<String>) -> Option<NormalizedHref> {
    if href.starts_with('/') {
        let parsed = split_fragment(href);
        return Some(NormalizedHref {
            normalized_path: normalize_internal_path(parsed.0),
            fragment: parsed.1,
        });
    }

    let url = Url::parse(href).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    if !host_matches(domains, &host) {
        return None;
    }
    let mut path = url.path().to_string();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    Some(NormalizedHref {
        normalized_path: normalize_internal_path(&path),
        fragment: url.fragment().map(|value| value.to_string()),
    })
}

fn host_matches(domains: &HashSet<String>, host: &str) -> bool {
    domains
        .iter()
        .any(|domain| host == domain || host.ends_with(&format!(".{domain}")))
}

fn normalize_internal_path(value: &str) -> String {
    let (path_part, fragment) = split_fragment(value);
    let (path, query) = match path_part.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (path_part, None),
    };
    let path = if path.trim().is_empty() {
        "/".to_string()
    } else {
        let trimmed = path.trim();
        if trimmed == "/" {
            "/".to_string()
        } else {
            format!("/{}", trimmed.trim_matches('/'))
        }
    };
    let mut normalized = path;
    if let Some(query) = query
        && !query.is_empty()
    {
        normalized.push('?');
        normalized.push_str(query);
    }
    if let Some(fragment) = fragment
        && !fragment.is_empty()
    {
        normalized.push('#');
        normalized.push_str(&fragment);
    }
    normalized
}

fn display_internal_href(value: &str) -> String {
    let normalized = normalize_internal_path(value);
    let (without_fragment, fragment) = split_fragment(&normalized);
    let (path, query) = match without_fragment.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (without_fragment, None),
    };
    let display_path = if path == "/" {
        "/".to_string()
    } else {
        format!("{}/", path.trim_end_matches('/'))
    };
    let mut display = display_path;
    if let Some(query) = query {
        display.push('?');
        display.push_str(query);
    }
    if let Some(fragment) = fragment {
        display.push('#');
        display.push_str(&fragment);
    }
    display
}

fn split_fragment(value: &str) -> (&str, Option<String>) {
    match value.split_once('#') {
        Some((before, fragment)) => (before, Some(fragment.to_string())),
        None => (value, None),
    }
}

fn asset_lookup_key(href: &str) -> Option<String> {
    if href.starts_with('/') {
        return href
            .rsplit('/')
            .next()
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase());
    }
    let parsed = Url::parse(href).ok()?;
    parsed
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn remote_image_url(href: &str) -> Option<String> {
    let parsed = Url::parse(href).ok()?;
    match parsed.scheme() {
        "http" | "https" => Some(parsed.to_string()),
        _ => None,
    }
}

fn unique_asset_match(matches: &[AssetLookup]) -> Option<AssetLookup> {
    let mut unique = matches
        .iter()
        .map(|value| {
            (
                value.asset_id,
                value.variant.clone(),
                value.asset_label.clone(),
            )
        })
        .collect::<HashSet<_>>();
    if unique.len() != 1 {
        return None;
    }
    let (asset_id, variant, asset_label) = unique.drain().next()?;
    Some(AssetLookup {
        asset_id,
        variant,
        asset_label,
    })
}

fn add_asset_lookup(map: &mut HashMap<String, Vec<AssetLookup>>, key: &str, lookup: AssetLookup) {
    map.entry(key.to_ascii_lowercase())
        .or_default()
        .push(lookup);
}

fn resolve_asset_render_candidate(
    asset: &entities::asset::Model,
    variants: &[entities::asset_variant::Model],
    variant_kind: &str,
) -> Option<AssetRenderCandidate> {
    if variant_kind == "original" {
        return Some(AssetRenderCandidate {
            filename: asset.storage_basename.clone(),
            width: asset.width,
            height: asset.height,
        });
    }

    variants
        .iter()
        .find(|variant| variant.variant_kind == variant_kind)
        .map(|variant| AssetRenderCandidate {
            filename: variant.filename.clone(),
            width: variant.width,
            height: variant.height,
        })
}

fn render_asset_figure_html(
    candidate: &AssetRenderCandidate,
    alt: &str,
    title: Option<&str>,
) -> String {
    let width_attr = candidate
        .width
        .filter(|value| *value > 0)
        .map(|value| format!(" width=\"{value}\""))
        .unwrap_or_default();
    let height_attr = candidate
        .height
        .filter(|value| *value > 0)
        .map(|value| format!(" height=\"{value}\""))
        .unwrap_or_default();
    let caption = title
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("<figcaption>{}</figcaption>", escape_html(value)))
        .unwrap_or_default();
    format!(
        "<figure><img src=\"/media/images/{}\" alt=\"{}\" loading=\"lazy\"{}{} />{}</figure>",
        escape_html(&candidate.filename),
        escape_html(alt),
        width_attr,
        height_attr,
        caption
    )
}

// TODO replace Regex parsing with a proper HTML parser somewhere
fn extract_html_attribute(attributes: &str, name: &str) -> Option<String> {
    let pattern = format!(
        r#"(?is)\b{}\s*=\s*(?:"([^"]*)"|'([^']*)')"#,
        regex::escape(name)
    );
    let regex = Regex::new(&pattern).ok()?;
    regex
        .captures(attributes)
        .and_then(|captures| captures.get(1).or_else(|| captures.get(2)))
        .map(|value| html_entity_decode(value.as_str()))
}

fn issue_id(kind: &str, start: usize, end: usize) -> String {
    format!("{kind}:{start}:{end}")
}

fn overlaps(occupied: &[(usize, usize)], start: usize, end: usize) -> bool {
    occupied
        .iter()
        .any(|(left, right)| start < *right && end > *left)
}

fn clip_snippet(source: &str, start: usize, end: usize) -> String {
    let snippet = source.get(start..end).unwrap_or("");
    let snippet_chars = snippet.chars().count();
    if snippet_chars <= 140 {
        snippet.to_string()
    } else {
        let truncated = snippet.chars().take(137).collect::<String>();
        format!("{truncated}...")
    }
}

fn inside_html_tag(source: &str, index: usize) -> bool {
    let left = source[..index].rfind('<');
    let right = source[..index].rfind('>');
    matches!((left, right), (Some(left), Some(right)) if left > right)
        || matches!((left, right), (Some(_), None))
}

fn escape_markdown_label(value: &str) -> String {
    value.replace(']', r"\]")
}

fn escape_shortcode_attr(value: &str) -> String {
    value.replace('\\', r"\\").replace('"', "\\\"")
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn html_entity_decode(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn markdown_link_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        #[allow(clippy::expect_used)]
        Regex::new(r#"(?m)(!?)\[([^\]\n]+)\]\(([^)\s]+)\)"#).expect("valid markdown link regex")
    })
}

fn html_anchor_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        #[allow(clippy::expect_used)]
        Regex::new(r#"(?is)<a\b[^>]*href\s*=\s*(?:["'])([^"']+)(?:["'])[^>]*>(.*?)</a>"#)
            .expect("valid html anchor regex")
    })
}

fn html_img_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        #[allow(clippy::expect_used)]
        Regex::new(r#"(?is)<img\b([^>]*)/?>"#).expect("valid html image regex")
    })
}

fn bare_url_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        #[allow(clippy::expect_used)]
        Regex::new(r#"https?://[^\s<>\"]+"#).expect("valid bare url regex")
    })
}

fn style_attr_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        #[allow(clippy::expect_used)]
        Regex::new(r#"(?is)\bstyle\s*=\s*(?:"[^"]*"|'[^']*')"#).expect("valid style regex")
    })
}

fn class_attr_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        #[allow(clippy::expect_used)]
        Regex::new(r#"(?is)\bclass\s*=\s*(?:"[^"]+"|'[^']+')"#).expect("valid class regex")
    })
}

fn dangerous_html_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        #[allow(clippy::expect_used)]
        Regex::new(r#"(?is)<\s*(script|iframe|object|embed|form)\b[^>]*>"#)
            .expect("valid dangerous html regex")
    })
}

fn complex_html_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        #[allow(clippy::expect_used)]
        Regex::new(r#"(?is)<\s*(div|section|table|ul|ol|li|blockquote|figure|figcaption)\b[^>]*>"#)
            .expect("valid complex html regex")
    })
}

fn asset_shortcode_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        #[allow(clippy::expect_used)]
        Regex::new(
            r#"\[\[asset id="([^"]+)" variant="([^"]+)" alt="([^"]*)"(?: title="([^"]*)")?\]\]"#,
        )
        .expect("valid asset shortcode regex")
    })
}

fn simple_tag_regex(tag: &str) -> Regex {
    #[allow(clippy::expect_used)]
    Regex::new(&format!(r#"(?is)<{tag}\b[^>]*>(.*?)</{tag}>"#)).expect("valid simple tag regex")
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::entities::{self, PageType};

    fn sample_content(
        id: Uuid,
        title: &str,
        slug: &str,
        body: &str,
    ) -> entities::content_item::Model {
        entities::content_item::Model {
            id,
            site_id: Uuid::now_v7(),
            page_type: PageType::Page,
            title: title.to_string(),
            slug: slug.to_string(),
            page_content: body.to_string(),
            draft: true,
            creator_sub: "creator".to_string(),
            created_at: Utc
                .with_ymd_and_hms(2026, 3, 10, 12, 0, 0)
                .single()
                .expect("valid date"),
            last_updated: None,
            published_at: None,
        }
    }

    fn context_for_tests() -> ScanContext {
        let mut link_targets = HashMap::new();
        link_targets.insert(
            normalize_internal_path("/legacy/path/"),
            LinkTarget {
                title: "Linked Page".to_string(),
                href: "/legacy/path/".to_string(),
            },
        );
        let mut asset_matches = HashMap::new();
        add_asset_lookup(
            &mut asset_matches,
            "header.png",
            AssetLookup {
                asset_id: Uuid::parse_str("019f8e3a-f4ba-7c91-9c8e-2fc19e4c16e1")
                    .expect("valid uuid"),
                asset_label: "header.png".to_string(),
                variant: "original".to_string(),
            },
        );
        ScanContext {
            internal_domains: HashSet::from_iter(["example.com".to_string()]),
            link_targets,
            asset_matches,
        }
    }

    #[test]
    fn parse_domain_list_deduplicates() {
        assert_eq!(
            parse_domain_list("example.com,\nwww.example.com\nexample.com"),
            vec!["example.com".to_string(), "www.example.com".to_string()]
        );
    }

    #[test]
    fn scans_and_rewrites_plain_internal_url() {
        let context = context_for_tests();
        let content = sample_content(
            Uuid::now_v7(),
            "Body",
            "body",
            "See https://example.com/legacy/path/ for more.",
        );
        let report = scan_content(&content, &context);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].kind, "bare_url");
        assert_eq!(
            report.issues[0].proposed_value.as_deref(),
            Some("[Linked Page](/legacy/path/)")
        );
    }

    #[test]
    fn scans_html_link_to_markdown() {
        let context = context_for_tests();
        let content = sample_content(
            Uuid::now_v7(),
            "Body",
            "body",
            "<a href=\"https://example.com/legacy/path/\">Read more</a>",
        );
        let report = scan_content(&content, &context);
        assert_eq!(report.issues[0].kind, "html_link");
        assert_eq!(
            report.issues[0].proposed_value.as_deref(),
            Some("[Read more](/legacy/path/)")
        );
    }

    #[test]
    fn scans_html_image_to_shortcode() {
        let context = context_for_tests();
        let content = sample_content(
            Uuid::now_v7(),
            "Body",
            "body",
            "<img src=\"https://example.com/uploads/header.png\" alt=\"Hero\" />",
        );
        let report = scan_content(&content, &context);
        assert_eq!(report.issues[0].kind, "html_image");
        assert!(
            report.issues[0]
                .proposed_value
                .as_deref()
                .unwrap_or_default()
                .starts_with("[[asset id=\"019f8e3a-f4ba-7c91-9c8e-2fc19e4c16e1\"")
        );
    }

    #[test]
    fn scans_inline_tags() {
        let context = context_for_tests();
        let content = sample_content(
            Uuid::now_v7(),
            "Body",
            "body",
            "<p>Para</p><strong>Bold</strong><b>More</b><i>Italic</i>",
        );
        let report = scan_content(&content, &context);
        let kinds = report
            .issues
            .into_iter()
            .map(|issue| issue.kind)
            .collect::<Vec<_>>();
        assert!(kinds.contains(&"paragraph".to_string()));
        assert!(kinds.contains(&"strong".to_string()));
        assert!(kinds.contains(&"bold".to_string()));
        assert!(kinds.contains(&"italic".to_string()));
    }

    #[test]
    fn scans_review_only_markup() {
        let context = context_for_tests();
        let content = sample_content(
            Uuid::now_v7(),
            "Body",
            "body",
            "<div class=\"hero\" style=\"color:red\"><script>alert(1)</script></div>",
        );
        let report = scan_content(&content, &context);
        let kinds = report
            .issues
            .into_iter()
            .map(|issue| issue.kind)
            .collect::<Vec<_>>();
        assert!(kinds.contains(&"inline_style".to_string()));
        assert!(kinds.contains(&"inline_class".to_string()));
        assert!(kinds.contains(&"dangerous_html".to_string()));
        assert!(kinds.contains(&"complex_html".to_string()));
    }

    #[test]
    fn renders_figure_markup() {
        let candidate = AssetRenderCandidate {
            filename: "photo.png".to_string(),
            width: Some(640),
            height: Some(480),
        };
        let html = render_asset_figure_html(&candidate, "Alt", Some("Caption"));
        assert!(html.contains("<figure><img"));
        assert!(html.contains("<figcaption>Caption</figcaption>"));
        assert!(html.contains("width=\"640\""));
        assert!(html.contains("height=\"480\""));
    }

    #[test]
    fn clip_snippet_handles_multibyte_characters() {
        let content = format!(
            "<a href=\"https://youtu.be/7IIBp1HQlWQ?si=C-oBsMI8ufTRXQpm\" data-type=\"link\" data-id=\"https://youtu.be/7IIBp1HQlWQ?si=C-oBsMI8ufTRXQpm\">{}</a>",
            "…".repeat(80)
        );
        let clipped = clip_snippet(&content, 0, content.len());
        assert!(clipped.ends_with("..."));
        assert!(clipped.starts_with("<a href="));
    }
}
