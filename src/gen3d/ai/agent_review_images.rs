use std::path::{Path, PathBuf};

pub(super) fn parse_paths_array_from_args(args: &serde_json::Value, keys: &[&str]) -> Vec<PathBuf> {
    for key in keys {
        let Some(arr) = args.get(*key).and_then(|v| v.as_array()) else {
            continue;
        };
        let mut out = Vec::new();
        for value in arr {
            let Some(s) = value.as_str().map(|s| s.trim()).filter(|s| !s.is_empty()) else {
                continue;
            };
            // Some models attempt to reference previous tool results using placeholders like
            // `$CALL_1.images[0]`. Gravimera does not support templating tool outputs into args.
            // Ignore these placeholders and fall back to the latest rendered images in cache.
            if s.starts_with('$') {
                continue;
            }
            out.push(PathBuf::from(s));
        }
        if !out.is_empty() {
            return out;
        }
    }
    Vec::new()
}

pub(super) fn parse_review_preview_images_from_args(args: &serde_json::Value) -> Vec<PathBuf> {
    parse_paths_array_from_args(
        args,
        &[
            "preview_images",
            "images",
            "image_paths",
            "paths",
            "preview_image_paths",
        ],
    )
}

fn is_user_reference_photo_path(path: &Path) -> bool {
    let mut last = None::<&str>;
    for component in path.components() {
        let Some(seg) = component.as_os_str().to_str() else {
            last = None;
            continue;
        };
        if last == Some("inputs") && seg == "images" {
            return true;
        }
        last = Some(seg);
    }
    false
}

pub(super) fn validate_review_images_for_llm(run_dir: &Path, paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let run_dir = std::fs::canonicalize(run_dir)
        .map_err(|err| format!("Failed to canonicalize run_dir {}: {err}", run_dir.display()))?;

    let mut out: Vec<PathBuf> = Vec::with_capacity(paths.len());
    for path in paths {
        let canonical = std::fs::canonicalize(path)
            .map_err(|err| format!("Invalid preview image path {}: {err}", path.display()))?;
        if !canonical.starts_with(&run_dir) {
            return Err(format!(
                "Refusing preview image outside Gen3D run cache. Expected under {} but got {}.",
                run_dir.display(),
                canonical.display()
            ));
        }
        if is_user_reference_photo_path(&canonical) {
            return Err(format!(
                "Refusing user reference photos as preview images (attempt_*/inputs/images/*): {}. Reference photos are summarized into text and not sent to the LLM.",
                canonical.display()
            ));
        }
        let ext = canonical
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if ext != "png" {
            return Err(format!(
                "Refusing non-PNG preview image: {} (expected a Gen3D render PNG).",
                canonical.display()
            ));
        }
        out.push(canonical);
    }
    Ok(out)
}

fn file_name_lower(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
}

fn is_motion_preview_image(path: &Path) -> bool {
    let Some(name) = file_name_lower(path) else {
        return false;
    };
    name.contains("move_sheet")
        || name.contains("attack_sheet")
        || name.contains("move_frame")
        || name.contains("attack_frame")
}

pub(super) fn motion_sheets_needed_from_smoke_results(
    smoke_results: &serde_json::Value,
) -> (bool, bool) {
    // Returns (include_move_sheet, include_attack_sheet).
    //
    // We rely on motion_validation's structured issue list rather than prompt heuristics so we can
    // be conservative with large motion-sheet images unless smoke_check has concrete errors.
    let motion_ok = smoke_results
        .get("motion_validation")
        .and_then(|v| v.get("ok"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let Some(issues) = smoke_results
        .get("motion_validation")
        .and_then(|v| v.get("issues"))
        .and_then(|v| v.as_array())
    else {
        // If validation failed but the issue list is missing/unparseable, fall back to including
        // the move sheet for extra visual context (it is usually the most informative).
        return (!motion_ok, false);
    };

    let mut include_move_sheet = false;
    let mut include_attack_sheet = false;
    for issue in issues {
        let severity = issue
            .get("severity")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if severity != "error" {
            continue;
        }

        match issue.get("channel").and_then(|v| v.as_str()).map(str::trim) {
            Some("attack_primary") => include_attack_sheet = true,
            Some("move") => include_move_sheet = true,
            Some(_) | None => include_move_sheet = true,
        }
    }

    (include_move_sheet || !motion_ok, include_attack_sheet)
}

pub(super) fn select_review_preview_images(
    preview_images: &[PathBuf],
    include_move_sheet: bool,
    include_attack_sheet: bool,
) -> Vec<PathBuf> {
    // Default policy for "routine" visual reviews:
    // - Prefer 5 static render views (front/left_back/right_back/top/bottom).
    // - Only include the relevant motion sheet(s) when smoke_check reports motion/attack issues.
    let preferred_static = [
        "render_front.png",
        "render_left_back.png",
        "render_right_back.png",
        "render_top.png",
        "render_bottom.png",
    ];

    let mut out: Vec<PathBuf> = Vec::new();
    for desired in preferred_static {
        if let Some(p) = preview_images
            .iter()
            .find(|p| file_name_lower(p).as_deref() == Some(desired))
        {
            out.push(p.clone());
        }
    }

    if out.is_empty() {
        for p in preview_images {
            if out.len() >= 5 {
                break;
            }
            if is_motion_preview_image(p) {
                continue;
            }
            out.push(p.clone());
        }
    }

    if out.is_empty() {
        out.extend(preview_images.iter().take(5).cloned());
    }

    if include_move_sheet
        && !out
            .iter()
            .any(|p| file_name_lower(p).as_deref() == Some("move_sheet.png"))
    {
        if let Some(p) = preview_images
            .iter()
            .find(|p| file_name_lower(p).as_deref() == Some("move_sheet.png"))
        {
            out.push(p.clone());
        }
    }
    if include_attack_sheet
        && !out
            .iter()
            .any(|p| file_name_lower(p).as_deref() == Some("attack_sheet.png"))
    {
        if let Some(p) = preview_images
            .iter()
            .find(|p| file_name_lower(p).as_deref() == Some("attack_sheet.png"))
        {
            out.push(p.clone());
        }
    }

    out
}

pub(super) fn review_capture_dimensions_for_max_dim(max_dim_px: u32) -> (u32, u32) {
    let size = max_dim_px.clamp(256, 4096) as f32;
    let base_w = super::super::GEN3D_REVIEW_CAPTURE_WIDTH_PX as f32;
    let base_h = super::super::GEN3D_REVIEW_CAPTURE_HEIGHT_PX as f32;
    let base_max = base_w.max(base_h).max(1.0);
    let scale = (size / base_max).max(1e-3);
    let w = (base_w * scale).round().clamp(256.0, 4096.0) as u32;
    let h = (base_h * scale).round().clamp(256.0, 4096.0) as u32;
    (w, h)
}
