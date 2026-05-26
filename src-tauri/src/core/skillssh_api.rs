//! Skills marketplace API client.
//!
//! Talks to the JSON API at `https://skills.ruoxhub.com/api/skills`. The module
//! name still reads `skillssh` for backward compatibility with call sites that
//! were originally written against the old `skills.sh` HTML scraper — the wire
//! format and host are now different but the public surface (`fetch_leaderboard`,
//! `search_skills`, `SkillsShSkill`) is unchanged.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const API_BASE: &str = "https://skills.ruoxhub.com/api/skills";
const CATEGORIES_API: &str = "https://skills.ruoxhub.com/api/categories";
/// The marketplace API caps `limit` at 20 per request — see the upstream
/// rejection of larger pages with an empty body. We page in chunks of this
/// size and concatenate the results.
const PAGE_LIMIT: usize = 20;
/// Browser-style headers required by the API; direct calls without these
/// headers are rejected with `Forbidden: Direct API access is not allowed`.
const REFERER: &str = "https://skills.ruoxhub.com/";
const USER_AGENT: &str =
    "Mozilla/5.0 (compatible; skills-manager) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsShSkill {
    /// Stable identifier `<source>/<skill_id>` so the existing install path
    /// (`install_from_skillssh(source, skill_id)`) keeps working unchanged.
    pub id: String,
    /// Skill directory name within the source repo — used as the install
    /// locator (`find_skill_dir` walks the repo looking for this name).
    pub skill_id: String,
    /// Human-readable name (defaults to `skill_id` when the API omits it).
    pub name: String,
    /// `<owner>/<repo>` shorthand. Combined with `skill_id` it yields the
    /// install reference and the GitHub URL the UI surfaces.
    pub source: String,
    /// Repo star count from the upstream API. Used for sorting/badges.
    /// Renamed to `installs` here only to preserve the existing TS field
    /// name on the frontend (`SkillsShSkill.installs`) — no behavior change.
    pub installs: u64,
    pub description: Option<String>,
    /// Chinese description supplied by the upstream marketplace. The frontend
    /// uses this in zh / zh-TW locales and falls back to `description` when
    /// missing — mirroring how `name`/`skill_id` already split locale-agnostic
    /// identifiers from human-facing copy.
    pub zh_desc: Option<String>,
    pub forks: u64,
    pub updated_at: Option<String>,
    pub author: Option<String>,
    pub author_avatar: Option<String>,
    pub github_url: Option<String>,
    pub branch: Option<String>,
    pub subpath: Option<String>,
    /// Category ids the upstream API attaches to this skill. Empty when
    /// uncategorised. Used by the frontend to highlight category badges and
    /// to cross-check the active category filter.
    #[serde(default)]
    pub categories: Vec<String>,
}

/// Category metadata returned by `/api/categories`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillCategory {
    pub id: String,
    pub name: String,
    /// Number of skills in this category, as reported by the upstream API.
    /// Used for the dropdown badge in the UI.
    pub skills_count: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum LeaderboardType {
    AllTime,
    Trending,
    Hot,
}

impl LeaderboardType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "trending" => Self::Trending,
            "hot" => Self::Hot,
            _ => Self::AllTime,
        }
    }

    /// Map UI tabs to the API's `sortBy` parameter. `Hot` falls back to
    /// `updatedAt` since the API has no dedicated "hot" sort — recently
    /// updated is the closest proxy.
    fn sort_by(&self) -> &'static str {
        match self {
            Self::AllTime => "stars",
            Self::Trending => "trending",
            Self::Hot => "updatedAt",
        }
    }
}

pub fn build_http_client(proxy_url: Option<&str>, timeout_secs: u64) -> reqwest::blocking::Client {
    let mut builder = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(timeout_secs));
    if let Some(proxy) = proxy_url.filter(|s| !s.is_empty()) {
        if let Ok(p) = reqwest::Proxy::all(proxy) {
            builder = builder.proxy(p);
        }
    }
    builder.build().unwrap_or_default()
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    skills: Option<Vec<ApiSkill>>,
}

#[derive(Debug, Deserialize)]
struct ApiSkill {
    name: Option<String>,
    description: Option<String>,
    /// Chinese description from the upstream marketplace. Optional — falls back
    /// to `description` on the frontend when missing.
    #[serde(default, alias = "zhDesc", alias = "zh_description")]
    zh_desc: Option<String>,
    #[serde(rename = "githubUrl")]
    github_url: Option<String>,
    #[serde(default)]
    stars: u64,
    #[serde(default)]
    forks: u64,
    #[serde(rename = "updatedAt")]
    updated_at: Option<String>,
    author: Option<String>,
    #[serde(rename = "authorAvatar")]
    author_avatar: Option<String>,
    path: Option<String>,
    branch: Option<String>,
    /// Categories the upstream API associates with this skill. Each entry has
    /// at least an `id`; we discard the (redundant) `name` here since the UI
    /// resolves it from `fetch_categories`.
    #[serde(default)]
    categories: Vec<ApiSkillCategoryRef>,
}

#[derive(Debug, Deserialize)]
struct ApiSkillCategoryRef {
    id: Option<String>,
}

/// Number of leaderboard entries to fetch up front. The API caps `limit` at
/// 20 per request, so this is `LEADERBOARD_TARGET / 20` round-trips per
/// (cached) leaderboard load. The frontend paginates these client-side at
/// 24/page, so 1200 entries → 50 UI pages, the supported browsing depth.
const LEADERBOARD_TARGET: usize = 1200;

pub fn fetch_leaderboard(
    board: LeaderboardType,
    proxy_url: Option<&str>,
    category: Option<&str>,
) -> Result<Vec<SkillsShSkill>> {
    fetch_pages(
        proxy_url,
        board.sort_by(),
        None,
        category,
        LEADERBOARD_TARGET,
    )
}

pub fn search_skills(
    query: &str,
    limit: usize,
    proxy_url: Option<&str>,
    category: Option<&str>,
) -> Result<Vec<SkillsShSkill>> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    fetch_pages(proxy_url, "stars", Some(trimmed), category, limit.max(1))
}

#[derive(Debug, Deserialize)]
struct CategoriesResponse {
    categories: Option<Vec<ApiCategory>>,
}

#[derive(Debug, Deserialize)]
struct ApiCategory {
    id: Option<String>,
    name: Option<String>,
    #[serde(default, rename = "skillsCount")]
    skills_count: u64,
}

pub fn fetch_categories(proxy_url: Option<&str>) -> Result<Vec<SkillCategory>> {
    let client = build_http_client(proxy_url, 15);
    let response = client
        .get(CATEGORIES_API)
        .header("Referer", REFERER)
        .header("Accept", "application/json")
        .send()
        .with_context(|| format!("Failed to call categories API ({CATEGORIES_API})"))?;

    if !response.status().is_success() {
        anyhow::bail!("Categories API returned HTTP {}", response.status());
    }

    let body: CategoriesResponse = response
        .json()
        .context("Failed to parse categories API response")?;

    Ok(body
        .categories
        .unwrap_or_default()
        .into_iter()
        .filter_map(|c| {
            let id = c.id.filter(|s| !s.is_empty())?;
            let name = c.name.filter(|s| !s.is_empty()).unwrap_or_else(|| id.clone());
            Some(SkillCategory {
                id,
                name,
                skills_count: c.skills_count,
            })
        })
        .collect())
}

fn fetch_pages(
    proxy_url: Option<&str>,
    sort_by: &str,
    search: Option<&str>,
    category: Option<&str>,
    target: usize,
) -> Result<Vec<SkillsShSkill>> {
    let client = build_http_client(proxy_url, 15);
    let mut out = Vec::with_capacity(target);
    let mut seen = HashSet::new();
    let mut page = 1usize;

    while out.len() < target {
        let remaining = target - out.len();
        let limit = remaining.min(PAGE_LIMIT);
        let skills = fetch_page(&client, sort_by, search, category, page, limit)?;
        if skills.is_empty() {
            break;
        }

        let mut added = 0usize;
        for skill in skills {
            if seen.insert(skill.id.clone()) {
                out.push(skill);
                added += 1;
                if out.len() >= target {
                    break;
                }
            }
        }

        // The API can return < `limit` on the last page, or repeat entries
        // we've already seen — both indicate we're done paging.
        if added == 0 {
            break;
        }
        page += 1;
    }

    Ok(out)
}

fn fetch_page(
    client: &reqwest::blocking::Client,
    sort_by: &str,
    search: Option<&str>,
    category: Option<&str>,
    page: usize,
    limit: usize,
) -> Result<Vec<SkillsShSkill>> {
    let mut url = format!("{API_BASE}?page={page}&limit={limit}&sortBy={sort_by}");
    if let Some(q) = search {
        url.push_str("&search=");
        url.push_str(&urlencoding::encode(q));
    }
    if let Some(cat) = category.filter(|s| !s.is_empty()) {
        url.push_str("&category=");
        url.push_str(&urlencoding::encode(cat));
    }

    let response = client
        .get(&url)
        .header("Referer", REFERER)
        .header("Accept", "application/json")
        .send()
        .with_context(|| format!("Failed to call skills API ({url})"))?;

    if !response.status().is_success() {
        anyhow::bail!("Skills API returned HTTP {}", response.status());
    }

    let body: ApiResponse = response
        .json()
        .context("Failed to parse skills API response")?;

    Ok(body
        .skills
        .unwrap_or_default()
        .into_iter()
        .filter_map(api_skill_to_market_skill)
        .collect())
}

fn api_skill_to_market_skill(item: ApiSkill) -> Option<SkillsShSkill> {
    let github_url = item.github_url?;
    let (owner, repo, branch_from_url, subpath_from_url) = parse_github_url(&github_url)?;
    let source = format!("{owner}/{repo}");

    // Prefer the explicit subpath (skill directory inside the repo) over the
    // file path returned in the API. `path` is usually `SKILL.md`, but the
    // skill-locator we want is the directory containing it, which we recover
    // from the `tree/<branch>/<dir>` portion of `githubUrl`.
    let subpath = subpath_from_url.or_else(|| {
        item.path.as_deref().and_then(|p| {
            let trimmed = p.trim_matches('/');
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("SKILL.md") {
                None
            } else {
                std::path::Path::new(trimmed)
                    .parent()
                    .map(|parent| parent.to_string_lossy().replace('\\', "/"))
                    .filter(|s| !s.is_empty())
            }
        })
    });

    let skill_id = subpath
        .as_deref()
        .and_then(|s| s.rsplit('/').next())
        .or(item.name.as_deref())
        .filter(|s| !s.is_empty())
        .unwrap_or(&repo)
        .to_string();

    let display_name = item
        .name
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| skill_id.clone());

    let categories = item
        .categories
        .into_iter()
        .filter_map(|c| c.id.filter(|s| !s.is_empty()))
        .collect();

    Some(SkillsShSkill {
        id: format!("{source}/{skill_id}"),
        skill_id,
        name: display_name,
        source,
        installs: item.stars,
        description: item.description.filter(|s| !s.is_empty()),
        zh_desc: item.zh_desc.filter(|s| !s.is_empty()),
        forks: item.forks,
        updated_at: item.updated_at.filter(|s| !s.is_empty()),
        author: item.author.filter(|s| !s.is_empty()),
        author_avatar: item.author_avatar.filter(|s| !s.is_empty()),
        github_url: Some(github_url),
        branch: item.branch.or(branch_from_url).filter(|s| !s.is_empty()),
        subpath,
        categories,
    })
}

/// Extract `(owner, repo, branch, subpath)` from a GitHub web URL of the form
/// `https://github.com/<owner>/<repo>(/tree/<branch>/<subpath>)?`.
fn parse_github_url(url: &str) -> Option<(String, String, Option<String>, Option<String>)> {
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))?
        .trim_end_matches('/');

    let mut parts = rest.split('/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.trim_end_matches(".git").to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }

    match parts.next() {
        Some("tree") | Some("blob") => {
            let branch = parts.next()?.to_string();
            let subpath: String = parts.collect::<Vec<_>>().join("/");
            let subpath = if subpath.is_empty() {
                None
            } else {
                Some(subpath)
            };
            Some((owner, repo, Some(branch), subpath))
        }
        _ => Some((owner, repo, None, None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repo_root_github_url() {
        let parsed = parse_github_url("https://github.com/openclaw/openclaw").unwrap();
        assert_eq!(parsed.0, "openclaw");
        assert_eq!(parsed.1, "openclaw");
        assert!(parsed.2.is_none());
        assert!(parsed.3.is_none());
    }

    #[test]
    fn parses_tree_github_url_with_subpath() {
        let parsed = parse_github_url(
            "https://github.com/openclaw/openclaw/tree/main/extensions/tavily/skills/tavily",
        )
        .unwrap();
        assert_eq!(parsed.0, "openclaw");
        assert_eq!(parsed.1, "openclaw");
        assert_eq!(parsed.2.as_deref(), Some("main"));
        assert_eq!(parsed.3.as_deref(), Some("extensions/tavily/skills/tavily"));
    }

    #[test]
    fn strips_dot_git_suffix() {
        let parsed = parse_github_url("https://github.com/openclaw/openclaw.git").unwrap();
        assert_eq!(parsed.1, "openclaw");
    }

    #[test]
    fn rejects_non_github_url() {
        assert!(parse_github_url("https://gitlab.com/foo/bar").is_none());
    }

    #[test]
    fn maps_api_skill_to_market_skill() {
        let api = ApiSkill {
            name: Some("tavily".to_string()),
            description: Some("Tavily web search".to_string()),
            zh_desc: Some("Tavily 联网搜索".to_string()),
            github_url: Some(
                "https://github.com/openclaw/openclaw/tree/main/extensions/tavily/skills/tavily"
                    .to_string(),
            ),
            stars: 100,
            forks: 10,
            updated_at: Some("2026-03-20T05:06:26.000Z".to_string()),
            author: Some("openclaw".to_string()),
            author_avatar: Some("https://example.com/avatar.png".to_string()),
            path: Some("SKILL.md".to_string()),
            branch: Some("main".to_string()),
            categories: vec![
                ApiSkillCategoryRef {
                    id: Some("automation-tools".to_string()),
                },
                ApiSkillCategoryRef { id: None },
                ApiSkillCategoryRef {
                    id: Some(String::new()),
                },
            ],
        };

        let skill = api_skill_to_market_skill(api).unwrap();
        assert_eq!(skill.id, "openclaw/openclaw/tavily");
        assert_eq!(skill.source, "openclaw/openclaw");
        assert_eq!(skill.skill_id, "tavily");
        assert_eq!(skill.name, "tavily");
        assert_eq!(skill.installs, 100);
        assert_eq!(skill.branch.as_deref(), Some("main"));
        assert_eq!(
            skill.subpath.as_deref(),
            Some("extensions/tavily/skills/tavily")
        );
        assert_eq!(skill.categories, vec!["automation-tools".to_string()]);
    }

    #[test]
    fn falls_back_to_repo_name_when_subpath_missing() {
        let api = ApiSkill {
            name: None,
            description: None,
            zh_desc: None,
            github_url: Some("https://github.com/foo/bar".to_string()),
            stars: 0,
            forks: 0,
            updated_at: None,
            author: None,
            author_avatar: None,
            path: None,
            branch: None,
            categories: Vec::new(),
        };

        let skill = api_skill_to_market_skill(api).unwrap();
        assert_eq!(skill.source, "foo/bar");
        assert_eq!(skill.skill_id, "bar");
        assert!(skill.categories.is_empty());
    }
}
