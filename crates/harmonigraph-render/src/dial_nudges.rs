//! Test fixtures discovered from the saved settings, including each array element.
use serde::{de::DeserializeOwned, Serialize};
use std::collections::BTreeMap;

fn numbers(value: &ron::Value, at: &str, out: &mut BTreeMap<String, ron::Value>) {
    match value {
        ron::Value::Map(map) => {
            for (key, value) in map.iter() {
                let ron::Value::String(key) = key else { panic!("settings key is not a name") };
                let path = if at.is_empty() { key.clone() } else { format!("{at}.{key}") };
                numbers(value, &path, out);
            }
        }
        ron::Value::Seq(items) => {
            for (i, item) in items.iter().enumerate() {
                numbers(item, &format!("{at}[{i}]"), out);
            }
        }
        ron::Value::Option(Some(value)) => numbers(value, at, out),
        ron::Value::Number(_) => {
            out.insert(at.to_owned(), value.clone());
        }
        _ => {}
    }
}

fn leaves(text: &str) -> BTreeMap<String, ron::Value> {
    let mut out = BTreeMap::new();
    numbers(&ron::from_str(text).unwrap(), "", &mut out);
    out
}

/// Change one numeric token in the serializer's own RON, preserving enums
/// (`ron::Value` can inspect them but cannot deserialize them back into settings).
/// Try 1% first, then inward or one discrete step if a bound or sanitizer snaps
/// it back. Every returned case must change the sanitized dial it names.
pub(crate) fn nudges<T: Serialize + DeserializeOwned>(
    base: T,
    sanitize: fn(T) -> T,
) -> Vec<(String, T)> {
    let text = ron::to_string(&sanitize(base)).unwrap();
    let original = leaves(&text);
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        match bytes[i] {
            b'"' => {
                i += 1;
                while bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
            }
            b'0'..=b'9' | b'-' => {
                i += 1;
                while i < bytes.len()
                    && matches!(bytes[i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
                {
                    i += 1;
                }
                spans.push(start..i);
            }
            _ => i += 1,
        }
    }
    assert_eq!(spans.len(), original.len(), "every numeric saved leaf needs a nudge");
    let mut out = Vec::new();
    for span in spans {
        let token = &text[span.clone()];
        let value: f64 = token.parse().unwrap();
        let integer = !token.contains(['.', 'e', 'E']);
        let mut found = None;
        for step in [0.01_f64, -0.01, 0.25, -0.25, 0.5, -0.5, 1.0, -1.0] {
            let changed = if integer {
                (value + step.signum()).to_string()
            } else {
                let scale = if value == 0.0 { 1.0 } else { value.abs() };
                format!("{:?}", value + scale * step)
            };
            let candidate = format!("{}{}{}", &text[..span.start], changed, &text[span.end..]);
            let Ok(settings) = ron::from_str::<T>(&candidate) else { continue };
            let settings = sanitize(settings);
            let after = leaves(&ron::to_string(&settings).unwrap());
            let paths: Vec<_> = original
                .iter()
                .filter(|(path, before)| after.get(*path) != Some(*before))
                .map(|(path, _)| path.clone())
                .collect();
            if paths.is_empty() {
                continue;
            }
            assert_eq!(paths.len(), 1, "one dial edit changed other saved dials: {paths:?}");
            found = Some((paths[0].clone(), settings));
            break;
        }
        out.push(
            found.unwrap_or_else(|| panic!("no sanitized nudge for token {token} at {span:?}")),
        );
    }
    let mut tested: Vec<_> = out.iter().map(|(path, _)| path.clone()).collect();
    tested.sort();
    assert_eq!(tested, original.into_keys().collect::<Vec<_>>());
    out
}

/// Distance between sampling offsets, with a repeating field's wrap removed.
pub(crate) fn apart(a: [f64; 2], b: [f64; 2], repeat: f64) -> f64 {
    (0..2)
        .map(|i| {
            let d = a[i] - b[i];
            if repeat > 0.0 {
                (d - repeat * (d / repeat).round()).abs()
            } else {
                d.abs()
            }
        })
        .fold(0.0, f64::max)
}
