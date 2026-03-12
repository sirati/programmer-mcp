//! JSON manipulation helpers for LSP response formatting.

/// Remove null values and empty arrays/objects from JSON to reduce noise.
pub fn strip_json_noise(val: serde_json::Value) -> serde_json::Value {
    match val {
        serde_json::Value::Object(map) => {
            let cleaned: serde_json::Map<String, serde_json::Value> = map
                .into_iter()
                .filter_map(|(k, v)| {
                    if v.is_null() {
                        return None;
                    }
                    let v = strip_json_noise(v);
                    if matches!(&v, serde_json::Value::Array(a) if a.is_empty()) {
                        return None;
                    }
                    if matches!(&v, serde_json::Value::Object(m) if m.is_empty()) {
                        return None;
                    }
                    Some((k, v))
                })
                .collect();
            serde_json::Value::Object(cleaned)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(strip_json_noise).collect())
        }
        other => other,
    }
}

/// Format JSON compactly: one entry per line at the top level.
pub fn format_compact_json(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::Array(arr) => arr
            .iter()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| v.to_string()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => serde_json::to_string(val).unwrap_or_else(|_| val.to_string()),
    }
}
