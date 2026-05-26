use std::sync::Arc;
use tauri::State;

use crate::core::{
    error::AppError,
    skill_store::SkillStore,
    skillssh_api::{self, LeaderboardType, SkillCategory, SkillsShSkill},
};

const LEADERBOARD_CACHE_TTL: i64 = 300; // 5 minutes
const CATEGORIES_CACHE_TTL: i64 = 3600; // 1 hour — categories change rarely
const CATEGORIES_CACHE_KEY: &str = "skill_categories_v1";

/// Build the per-board cache key, namespaced by category so a switch between
/// "all" and a specific category id doesn't return stale entries.
fn leaderboard_cache_key(board: &str, category: Option<&str>) -> String {
    match category.filter(|c| !c.is_empty()) {
        Some(cat) => format!("leaderboard_{board}_cat_{cat}"),
        None => format!("leaderboard_{board}"),
    }
}

#[tauri::command]
pub async fn fetch_leaderboard(
    board: String,
    category: Option<String>,
    store: State<'_, Arc<SkillStore>>,
) -> Result<Vec<SkillsShSkill>, AppError> {
    let category_for_key = category.as_deref().filter(|s| !s.is_empty());
    let cache_key = leaderboard_cache_key(&board, category_for_key);

    // Check cache
    if let Ok(Some(cached)) = store.get_cache(&cache_key, LEADERBOARD_CACHE_TTL) {
        if let Ok(skills) = serde_json::from_str::<Vec<SkillsShSkill>>(&cached) {
            return Ok(skills);
        }
    }

    let proxy_url = store.proxy_url();
    let board_type = LeaderboardType::from_str(&board);
    let category_owned = category_for_key.map(|s| s.to_string());
    let skills = tauri::async_runtime::spawn_blocking(move || {
        skillssh_api::fetch_leaderboard(
            board_type,
            proxy_url.as_deref(),
            category_owned.as_deref(),
        )
        .map_err(AppError::network)
    })
    .await??;

    // Update cache
    if let Ok(json) = serde_json::to_string(&skills) {
        store.set_cache(&cache_key, &json).ok();
    }

    Ok(skills)
}

#[tauri::command]
pub async fn search_skillssh(
    query: String,
    limit: Option<usize>,
    category: Option<String>,
    store: State<'_, Arc<SkillStore>>,
) -> Result<Vec<SkillsShSkill>, AppError> {
    let proxy_url = store.proxy_url();
    let requested = limit.unwrap_or(60);
    let bounded = requested.clamp(1, 300);
    let category_owned = category.filter(|s| !s.is_empty());
    tauri::async_runtime::spawn_blocking(move || {
        skillssh_api::search_skills(
            &query,
            bounded,
            proxy_url.as_deref(),
            category_owned.as_deref(),
        )
        .map_err(AppError::network)
    })
    .await?
}

#[tauri::command]
pub async fn fetch_skill_categories(
    store: State<'_, Arc<SkillStore>>,
) -> Result<Vec<SkillCategory>, AppError> {
    if let Ok(Some(cached)) = store.get_cache(CATEGORIES_CACHE_KEY, CATEGORIES_CACHE_TTL) {
        if let Ok(categories) = serde_json::from_str::<Vec<SkillCategory>>(&cached) {
            return Ok(categories);
        }
    }

    let proxy_url = store.proxy_url();
    let categories = tauri::async_runtime::spawn_blocking(move || {
        skillssh_api::fetch_categories(proxy_url.as_deref()).map_err(AppError::network)
    })
    .await??;

    if let Ok(json) = serde_json::to_string(&categories) {
        store.set_cache(CATEGORIES_CACHE_KEY, &json).ok();
    }

    Ok(categories)
}
