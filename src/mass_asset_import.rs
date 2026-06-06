use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use tokio::fs;
use url::Url;
use uuid::Uuid;

use crate::entities;
use crate::errors::SiteError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MassAssetOccurrence {
    pub content_id: Uuid,
    pub title: String,
    pub start: usize,
    pub end: usize,
    pub original_link: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MassAssetAffectedContent {
    pub content_id: Uuid,
    pub title: String,
    pub occurrence_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingAssetGroup {
    pub normalized_path: String,
    pub occurrence_count: usize,
    pub affected_content: Vec<MassAssetAffectedContent>,
    pub occurrences: Vec<MassAssetOccurrence>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LocalAssetCandidateRank {
    PathSuffix,
    Filename,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalAssetCandidate {
    pub absolute_path: PathBuf,
    pub relative_path: PathBuf,
    pub rank: LocalAssetCandidateRank,
    pub byte_length: u64,
}

pub fn normalize_asset_link(raw: &str, internal_domains: &[String]) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(parsed) = Url::parse(trimmed) {
        let host = parsed.host_str()?.to_ascii_lowercase();
        if !host_matches(internal_domains, &host) {
            return None;
        }
        return normalize_asset_path(parsed.path());
    }

    normalize_asset_path(trimmed)
}

pub fn find_missing_asset_groups(
    mut content_items: Vec<entities::content_item::Model>,
    internal_domains: &[String],
    limit: usize,
) -> Vec<MissingAssetGroup> {
    content_items.sort_by(|left, right| {
        right
            .last_updated
            .unwrap_or(right.created_at)
            .cmp(&left.last_updated.unwrap_or(left.created_at))
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
    });

    let mut groups = Vec::<MissingAssetGroup>::new();
    let mut group_indexes = HashMap::<String, usize>::new();

    for content in content_items {
        for link in extract_asset_links(&content.page_content, internal_domains) {
            let group_index = if let Some(group_index) = group_indexes.get(&link.normalized_path) {
                *group_index
            } else {
                if groups.len() >= limit {
                    continue;
                }
                let index = groups.len();
                group_indexes.insert(link.normalized_path.clone(), index);
                groups.push(MissingAssetGroup {
                    normalized_path: link.normalized_path.clone(),
                    occurrence_count: 0,
                    affected_content: Vec::new(),
                    occurrences: Vec::new(),
                });
                index
            };

            let Some(group) = groups.get_mut(group_index) else {
                continue;
            };
            group.occurrence_count = group.occurrence_count.saturating_add(1);
            group.occurrences.push(MassAssetOccurrence {
                content_id: content.id,
                title: content.title.clone(),
                start: link.start,
                end: link.end,
                original_link: link.original_link,
            });
            if let Some(row) = group
                .affected_content
                .iter_mut()
                .find(|row| row.content_id == content.id)
            {
                row.occurrence_count = row.occurrence_count.saturating_add(1);
            } else {
                group.affected_content.push(MassAssetAffectedContent {
                    content_id: content.id,
                    title: content.title.clone(),
                    occurrence_count: 1,
                });
            }
        }
    }

    groups
}

pub async fn find_local_asset_candidates(
    import_root: &Path,
    normalized_path: &str,
    limit: usize,
) -> Result<Vec<LocalAssetCandidate>, SiteError> {
    let canonical_root = fs::canonicalize(import_root).await.map_err(|error| {
        SiteError::BadRequest(format!("failed to read mass import path: {error}"))
    })?;
    let normalized_relative = normalized_path.trim_start_matches('/').to_ascii_lowercase();
    let filename = Path::new(normalized_path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();

    let mut candidates = Vec::new();
    let mut pending = vec![canonical_root.clone()];
    while let Some(directory) = pending.pop() {
        let mut entries = fs::read_dir(&directory).await.map_err(SiteError::from)?;
        while let Some(entry) = entries.next_entry().await.map_err(SiteError::from)? {
            let file_type = entry.file_type().await.map_err(SiteError::from)?;
            let path = entry.path();
            if file_type.is_dir() {
                let Ok(canonical) = fs::canonicalize(&path).await else {
                    continue;
                };
                if canonical.starts_with(&canonical_root) {
                    pending.push(canonical);
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let Ok(candidate) =
                build_candidate(&canonical_root, &path, &normalized_relative, &filename).await
            else {
                continue;
            };
            if let Some(candidate) = candidate {
                candidates.push(candidate);
            }
        }
    }

    candidates.sort_by(|left, right| {
        left.rank
            .cmp(&right.rank)
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });
    candidates.truncate(limit);
    Ok(candidates)
}

pub async fn validate_import_candidate(
    import_root: &Path,
    candidate_path: &Path,
) -> Result<PathBuf, SiteError> {
    let canonical_root = fs::canonicalize(import_root).await.map_err(|error| {
        SiteError::BadRequest(format!("failed to read mass import path: {error}"))
    })?;
    let canonical_candidate = fs::canonicalize(candidate_path).await.map_err(|error| {
        SiteError::BadRequest(format!("failed to read import candidate: {error}"))
    })?;
    if !canonical_candidate.starts_with(&canonical_root) {
        return Err(SiteError::BadRequest(
            "import candidate is outside the configured import path".to_string(),
        ));
    }
    let metadata = fs::metadata(&canonical_candidate)
        .await
        .map_err(SiteError::from)?;
    if !metadata.is_file() {
        return Err(SiteError::BadRequest(
            "import candidate is not a file".to_string(),
        ));
    }
    if !path_has_image_extension(&canonical_candidate) {
        return Err(SiteError::BadRequest(
            "import candidate is not a supported image file".to_string(),
        ));
    }
    Ok(canonical_candidate)
}

#[derive(Clone, Debug)]
struct ExtractedAssetLink {
    normalized_path: String,
    start: usize,
    end: usize,
    original_link: String,
}

fn extract_asset_links(source: &str, internal_domains: &[String]) -> Vec<ExtractedAssetLink> {
    let mut links = Vec::new();
    collect_regex_links(
        source,
        markdown_link_regex(),
        &[2],
        internal_domains,
        &mut links,
    );
    collect_regex_links(
        source,
        html_img_regex(),
        &[1, 2],
        internal_domains,
        &mut links,
    );
    collect_regex_links(
        source,
        html_anchor_regex(),
        &[1, 2],
        internal_domains,
        &mut links,
    );
    links.sort_by_key(|link| link.start);
    links
}

fn collect_regex_links(
    source: &str,
    regex: Option<&Regex>,
    capture_indexes: &[usize],
    internal_domains: &[String],
    links: &mut Vec<ExtractedAssetLink>,
) {
    let Some(regex) = regex else {
        return;
    };

    for captures in regex.captures_iter(source) {
        let href = capture_indexes
            .iter()
            .find_map(|capture_index| captures.get(*capture_index));
        let Some(href) = href else {
            continue;
        };
        let Some(normalized_path) = normalize_asset_link(href.as_str(), internal_domains) else {
            continue;
        };
        links.push(ExtractedAssetLink {
            normalized_path,
            start: captures
                .get(0)
                .map(|value| value.start())
                .unwrap_or(href.start()),
            end: captures
                .get(0)
                .map(|value| value.end())
                .unwrap_or(href.end()),
            original_link: href.as_str().to_string(),
        });
    }
}

async fn build_candidate(
    canonical_root: &Path,
    path: &Path,
    normalized_relative: &str,
    filename: &str,
) -> Result<Option<LocalAssetCandidate>, SiteError> {
    let canonical_path = validate_import_candidate(canonical_root, path).await?;
    let relative_path = canonical_path
        .strip_prefix(canonical_root)
        .map_err(|error| SiteError::internal(format!("failed to strip import root: {error}")))?
        .to_path_buf();
    let relative_string = relative_path.to_string_lossy().replace('\\', "/");
    let relative_lower = relative_string.to_ascii_lowercase();
    let candidate_filename = canonical_path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let rank = if relative_lower == normalized_relative
        || relative_lower.ends_with(&format!("/{normalized_relative}"))
    {
        Some(LocalAssetCandidateRank::PathSuffix)
    } else if !filename.is_empty() && candidate_filename == filename {
        Some(LocalAssetCandidateRank::Filename)
    } else {
        None
    };
    let Some(rank) = rank else {
        return Ok(None);
    };
    let metadata = fs::metadata(&canonical_path)
        .await
        .map_err(SiteError::from)?;
    Ok(Some(LocalAssetCandidate {
        absolute_path: canonical_path,
        relative_path,
        rank,
        byte_length: metadata.len(),
    }))
}

fn normalize_asset_path(raw: &str) -> Option<String> {
    let path = raw
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(raw)
        .split_once('#')
        .map(|(path, _)| path)
        .unwrap_or(raw)
        .trim();
    if path.is_empty() || !string_has_image_extension(path) {
        return None;
    }
    Some(format!("/{}", path.trim_start_matches('/')))
}

fn host_matches(internal_domains: &[String], host: &str) -> bool {
    internal_domains.iter().any(|domain| {
        let normalized = domain.trim().trim_start_matches('.').to_ascii_lowercase();
        !normalized.is_empty() && (host == normalized || host.ends_with(&format!(".{normalized}")))
    })
}

fn string_has_image_extension(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg"
            )
        })
        .unwrap_or(false)
}

fn path_has_image_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg"
            )
        })
        .unwrap_or(false)
}

fn markdown_link_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r#"(?s)!?\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)"#))
        .as_ref()
        .ok()
}

fn html_img_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r#"(?is)<img\b[^>]*\bsrc\s*=\s*(?:"([^"]+)"|'([^']+)')[^>]*>"#))
        .as_ref()
        .ok()
}

fn html_anchor_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Result<Regex, regex::Error>> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r#"(?is)<a\b[^>]*\bhref\s*=\s*(?:"([^"]+)"|'([^']+)')[^>]*>"#))
        .as_ref()
        .ok()
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use uuid::Uuid;

    use crate::entities;
    use crate::entities::PageType;

    fn content(title: &str, body: &str, created_at: &str) -> entities::content_item::Model {
        entities::content_item::Model {
            id: Uuid::now_v7(),
            site_id: Uuid::now_v7(),
            page_type: PageType::Post,
            title: title.to_string(),
            slug: title.to_lowercase().replace(' ', "-"),
            page_content: body.to_string(),
            draft: false,
            creator_sub: "tester".to_string(),
            created_at: DateTime::parse_from_rfc3339(created_at)
                .expect("invalid created_at")
                .with_timezone(&Utc),
            last_updated: None,
            published_at: None,
        }
    }

    #[test]
    fn normalizes_internal_and_relative_image_links_to_root_relative_paths() {
        let domains = vec!["example.com".to_string()];

        assert_eq!(
            super::normalize_asset_link("/wp-content/uploads/hero.jpg", &domains),
            Some("/wp-content/uploads/hero.jpg".to_string())
        );
        assert_eq!(
            super::normalize_asset_link("wp-content/uploads/hero.jpg", &domains),
            Some("/wp-content/uploads/hero.jpg".to_string())
        );
        assert_eq!(
            super::normalize_asset_link(
                "https://www.example.com/wp-content/uploads/hero.jpg?size=large",
                &domains
            ),
            Some("/wp-content/uploads/hero.jpg".to_string())
        );
        assert_eq!(
            super::normalize_asset_link(
                "https://external.example.net/wp-content/uploads/hero.jpg",
                &domains
            ),
            None
        );
    }

    #[test]
    fn groups_reused_asset_paths_newest_content_first() {
        let domains = vec!["example.com".to_string()];
        let older = content(
            "Older",
            r#"<p><img src="https://example.com/uploads/shared.jpg" /></p>"#,
            "2026-01-01T00:00:00Z",
        );
        let newer = content(
            "Newer",
            r#"[download](uploads/shared.jpg) ![Hero](/uploads/other.png)"#,
            "2026-01-03T00:00:00Z",
        );
        let newest = content(
            "Newest",
            r#"![Shared](/uploads/shared.jpg)"#,
            "2026-01-05T00:00:00Z",
        );

        let groups = super::find_missing_asset_groups(vec![older, newer, newest], &domains, 10);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].normalized_path, "/uploads/shared.jpg");
        assert_eq!(groups[0].occurrence_count, 3);
        assert_eq!(
            groups[0]
                .affected_content
                .iter()
                .map(|item| item.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Newest", "Newer", "Older"]
        );
        assert_eq!(groups[1].normalized_path, "/uploads/other.png");
    }

    #[tokio::test]
    async fn ranks_path_suffix_matches_before_filename_matches() {
        let temp = tempfile::tempdir().expect("failed to create temp dir");
        let import_root = temp.path();
        tokio::fs::create_dir_all(import_root.join("wp-content/uploads/2024"))
            .await
            .expect("failed to create nested dir");
        tokio::fs::create_dir_all(import_root.join("elsewhere"))
            .await
            .expect("failed to create fallback dir");
        tokio::fs::write(
            import_root.join("wp-content/uploads/2024/hero.jpg"),
            b"full",
        )
        .await
        .expect("failed to write full match");
        tokio::fs::write(import_root.join("elsewhere/hero.jpg"), b"name")
            .await
            .expect("failed to write name match");

        let matches = super::find_local_asset_candidates(
            import_root,
            "/wp-content/uploads/2024/hero.jpg",
            10,
        )
        .await
        .expect("failed to find candidates");

        assert_eq!(matches.len(), 2);
        assert_eq!(
            matches[0]
                .relative_path
                .to_str()
                .expect("path should be utf-8"),
            "wp-content/uploads/2024/hero.jpg"
        );
        assert_eq!(matches[0].rank, super::LocalAssetCandidateRank::PathSuffix);
        assert_eq!(matches[1].rank, super::LocalAssetCandidateRank::Filename);
    }

    #[tokio::test]
    async fn rejects_candidates_outside_import_root() {
        let temp = tempfile::tempdir().expect("failed to create temp dir");
        let import_root = temp.path().join("import");
        let outside = temp.path().join("outside");
        tokio::fs::create_dir_all(&import_root)
            .await
            .expect("failed to create import root");
        tokio::fs::create_dir_all(&outside)
            .await
            .expect("failed to create outside dir");
        tokio::fs::write(outside.join("hero.jpg"), b"outside")
            .await
            .expect("failed to write outside file");

        let err = super::validate_import_candidate(&import_root, &outside.join("hero.jpg"))
            .await
            .expect_err("outside candidate should be rejected");

        assert!(err.to_string().contains("outside"));
    }
}
