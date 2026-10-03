use ayran_core::{
    config::ConfigLayers, diagnostic::Diagnostic, harness::Harness, marketplace::Binding,
};
use serde_json::{Value, json};

pub fn rows(layers: &ConfigLayers, harnesses: &[Harness]) -> Result<Vec<Value>, Diagnostic> {
    let store = if layers.marketplaces.values().any(|m| m.value.project) {
        crate::trust::Store::read()?
    } else {
        crate::trust::Store::default()
    };
    layers.marketplaces.iter().map(|(name, marketplace)| {
        let bindings: serde_json::Map<_, _> = harnesses.iter().map(|h| {
            let value = match marketplace.value.binding(*h) {
                Some(Binding::Source(d)) => json!({"kind":"source", "source":d.source.label(), "ref":d.reference, "name":d.name.as_deref().unwrap_or(name)}),
                Some(Binding::Absent) => json!({"kind":"absent"}),
                None => Value::Null,
            };
            (h.binary().to_owned(), value)
        }).collect();
        Ok(json!({"name":name, "layer":marketplace.path, "trusted": !marketplace.value.project || store.trusted(&marketplace.path)?, "bindings":bindings}))
    }).collect()
}
pub fn print(rows: &[Value], harnesses: &[Harness]) {
    let mut header = vec!["name".into(), "layer".into(), "trusted".into()];
    header.extend(harnesses.iter().map(|h| h.binary().to_owned()));
    crate::list_command::print_row(header);
    for row in rows {
        let mut cells = vec![
            row["name"].as_str().unwrap().into(),
            row["layer"].as_str().unwrap().into(),
            row["trusted"].to_string(),
        ];
        cells.extend(harnesses.iter().map(|h| {
            let binding = &row["bindings"][h.binary()];
            match binding["kind"].as_str() {
                Some("source") => format!(
                    "{}{} (name: {})",
                    binding["source"].as_str().unwrap(),
                    binding["ref"]
                        .as_str()
                        .map(|r| format!(" ref: {r}"))
                        .unwrap_or_default(),
                    binding["name"].as_str().unwrap()
                ),
                Some("absent") => "—".into(),
                _ => "✗".into(),
            }
        }));
        crate::list_command::print_row(cells);
    }
}
