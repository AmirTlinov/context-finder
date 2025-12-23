use crate::command::domain::RequestOptions;

pub fn is_active(options: &RequestOptions) -> bool {
    !options.include_paths.is_empty()
        || !options.exclude_paths.is_empty()
        || options
            .file_pattern
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .is_some()
}

pub fn path_allowed(rel_path: &str, options: &RequestOptions) -> bool {
    let rel_path = rel_path.replace('\\', "/");

    if !options.include_paths.is_empty()
        && !options
            .include_paths
            .iter()
            .any(|p| path_prefix_matches(p, &rel_path))
    {
        return false;
    }

    if options
        .exclude_paths
        .iter()
        .any(|p| path_prefix_matches(p, &rel_path))
    {
        return false;
    }

    matches_file_pattern(
        &rel_path,
        options
            .file_pattern
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty()),
    )
}

fn normalize_filter_path(raw: &str) -> String {
    let mut value = raw.trim().replace('\\', "/");
    while value.starts_with("./") {
        value = value[2..].to_string();
    }
    value.trim_matches('/').to_string()
}

fn path_prefix_matches(prefix: &str, path: &str) -> bool {
    let prefix = normalize_filter_path(prefix);
    if prefix.is_empty() {
        return false;
    }
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

fn matches_file_pattern(path: &str, pattern: Option<&str>) -> bool {
    let Some(pattern) = pattern else {
        return true;
    };

    if !pattern.contains('*') && !pattern.contains('?') {
        return path.contains(pattern);
    }

    glob::Pattern::new(pattern)
        .map(|p| p.matches(path))
        .unwrap_or(false)
}
