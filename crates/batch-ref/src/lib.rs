const MAX_DEPTH: usize = 64;

fn decode_pointer_token(token: &str) -> Result<String, String> {
    let mut out = String::with_capacity(token.len());
    let mut chars = token.chars();
    while let Some(ch) = chars.next() {
        if ch != '~' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('0') => out.push('~'),
            Some('1') => out.push('/'),
            Some(other) => return Err(format!("Invalid JSON pointer escape '~{other}'")),
            None => return Err("Invalid JSON pointer escape '~'".to_string()),
        }
    }
    Ok(out)
}

fn resolve_json_pointer<'a>(
    root: &'a serde_json::Value,
    pointer: &str,
) -> Result<&'a serde_json::Value, String> {
    let pointer = pointer.strip_prefix('#').unwrap_or(pointer);
    if pointer.is_empty() {
        return Ok(root);
    }
    if !pointer.starts_with('/') {
        return Err(format!(
            "$ref must be a JSON pointer starting with '#/' or '/': got {pointer:?}"
        ));
    }

    let mut tokens = Vec::new();
    for raw in pointer.split('/').skip(1) {
        tokens.push(decode_pointer_token(raw)?);
    }

    if tokens.len() >= 3 && tokens[0] == "items" && tokens[2] == "data" {
        if let Some(item) = root.get("items").and_then(|v| v.get(&tokens[1])) {
            if item.get("status").and_then(|v| v.as_str()) == Some("error") {
                let msg = item
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                return Err(format!("$ref points to failed item '{}': {msg}", tokens[1]));
            }
        }
    }

    let mut current = root;
    for token in tokens {
        match current {
            serde_json::Value::Object(map) => {
                current = map
                    .get(&token)
                    .ok_or_else(|| format!("$ref path {pointer:?} not found at key {token:?}"))?;
            }
            serde_json::Value::Array(arr) => {
                let idx: usize = token.parse().map_err(|_| {
                    format!("$ref path {pointer:?} expected array index, got {token:?}")
                })?;
                current = arr.get(idx).ok_or_else(|| {
                    format!("$ref path {pointer:?} array index out of bounds: {idx}")
                })?;
            }
            _ => {
                return Err(format!(
                    "$ref path {pointer:?} reached non-container before token {token:?}"
                ));
            }
        }
    }

    Ok(current)
}

fn resolve_inner(
    value: serde_json::Value,
    ctx: &serde_json::Value,
    depth: usize,
) -> Result<serde_json::Value, String> {
    if depth > MAX_DEPTH {
        return Err("Ref resolution exceeded max depth".to_string());
    }

    match value {
        serde_json::Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(resolve_inner(item, ctx, depth + 1)?);
            }
            Ok(serde_json::Value::Array(out))
        }
        serde_json::Value::Object(map) => {
            let default_value = map.get("$default").cloned();
            let is_ref_wrapper = map.contains_key("$ref")
                && (map.len() == 1 || (map.len() == 2 && default_value.is_some()));

            if is_ref_wrapper {
                let pointer = map
                    .get("$ref")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "$ref must be a string".to_string())?;

                match resolve_json_pointer(ctx, pointer) {
                    Ok(found) => resolve_inner(found.clone(), ctx, depth + 1),
                    Err(err) => {
                        if let Some(default) = default_value {
                            return resolve_inner(default, ctx, depth + 1);
                        }
                        Err(err)
                    }
                }
            } else {
                let mut out = serde_json::Map::new();
                for (key, value) in map {
                    out.insert(key, resolve_inner(value, ctx, depth + 1)?);
                }
                Ok(serde_json::Value::Object(out))
            }
        }
        other => Ok(other),
    }
}

pub fn resolve_batch_refs(
    input: serde_json::Value,
    ctx: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    resolve_inner(input, ctx, 0)
}

#[cfg(test)]
mod tests {
    use super::resolve_batch_refs;

    #[test]
    fn resolves_simple_pointer() {
        let ctx = serde_json::json!({
            "items": {
                "a": { "status": "ok", "data": { "value": 1 } }
            }
        });
        let input = serde_json::json!({ "x": { "$ref": "#/items/a/data/value" } });
        let out = resolve_batch_refs(input, &ctx).expect("ok");
        assert_eq!(out["x"], 1);
    }

    #[test]
    fn uses_default_on_missing_pointer() {
        let ctx = serde_json::json!({ "items": {} });
        let input = serde_json::json!({ "x": { "$ref": "#/items/missing/data", "$default": 42 } });
        let out = resolve_batch_refs(input, &ctx).expect("ok");
        assert_eq!(out["x"], 42);
    }

    #[test]
    fn rejects_ref_to_failed_item_data() {
        let ctx = serde_json::json!({
            "items": {
                "bad": { "status": "error", "message": "nope", "data": null }
            }
        });
        let input = serde_json::json!({ "x": { "$ref": "#/items/bad/data" } });
        let err = resolve_batch_refs(input, &ctx).expect_err("expected error");
        assert!(err.contains("points to failed item"));
    }

    #[test]
    fn does_not_treat_objects_with_extra_keys_as_ref_wrappers() {
        let ctx = serde_json::json!({
            "items": {
                "a": { "status": "ok", "data": { "value": 1 } }
            }
        });
        let input = serde_json::json!({ "x": { "$ref": "#/items/a/data/value", "other": 1 } });
        let out = resolve_batch_refs(input, &ctx).expect("ok");
        assert_eq!(out["x"]["$ref"], "#/items/a/data/value");
        assert_eq!(out["x"]["other"], 1);
    }
}
