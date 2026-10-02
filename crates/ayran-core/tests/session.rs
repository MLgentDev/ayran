use ayran_core::{resolve::Request, session::merge};

#[test]
fn new_selections_and_disables_cancel_the_recorded_opposite() {
    let stored = Request {
        skills: vec!["vanished".into(), "keep".into()],
        no_plugins: vec!["p".into()],
        model: Some("old".into()),
        no_defaults: true,
        ..Default::default()
    };
    let flags = Request {
        no_skills: vec!["vanished".into()],
        plugins: vec!["p".into(), "p".into()],
        model: Some("new".into()),
        ..Default::default()
    };
    let merged = merge(&stored, flags);
    assert_eq!(merged.skills, ["keep"]);
    assert_eq!(merged.no_skills, ["vanished"]);
    assert_eq!(merged.plugins, ["p"]);
    assert!(merged.no_plugins.is_empty());
    assert_eq!(merged.model.as_deref(), Some("new"));
    assert!(merged.no_defaults);
}

#[test]
fn session_prefixes_ignore_hyphens_and_case() {
    use ayran_core::session::matches_prefix;
    let id = "aBcD1234-5678-4000-8000-000000000000";
    assert!(matches_prefix(id, "ABCD123456"));
    assert!(matches_prefix(id, "abcd1234-5678-4000-8000-000000000000"));
    assert!(!matches_prefix(id, "abcd1235"));
    assert!(!matches_prefix(id, ""));
}

#[test]
fn merge_applies_opposite_cancellation_to_every_capability_kind() {
    let stored = Request {
        plugins: vec!["old".into()],
        skills: vec!["old".into()],
        mcp: vec!["old".into()],
        profiles: vec!["old".into()],
        no_plugins: vec!["new".into()],
        no_skills: vec!["new".into()],
        no_mcp: vec!["new".into()],
        no_profiles: vec!["new".into()],
        ..Default::default()
    };
    let flags = Request {
        plugins: vec!["new".into()],
        skills: vec!["new".into()],
        mcp: vec!["new".into()],
        profiles: vec!["new".into()],
        no_plugins: vec!["old".into()],
        no_skills: vec!["old".into()],
        no_mcp: vec!["old".into()],
        no_profiles: vec!["old".into()],
        no_defaults: true,
        ..Default::default()
    };
    let merged = merge(&stored, flags);
    for selected in [
        &merged.plugins,
        &merged.skills,
        &merged.mcp,
        &merged.profiles,
    ] {
        assert_eq!(selected, &["new"]);
    }
    for disabled in [
        &merged.no_plugins,
        &merged.no_skills,
        &merged.no_mcp,
        &merged.no_profiles,
    ] {
        assert_eq!(disabled, &["old"]);
    }
    assert!(merged.no_defaults);
}

#[test]
fn normalized_records_keep_typed_order_and_never_store_harness_or_passthrough() {
    use ayran_core::{
        harness::{Effort, Harness},
        session::normalize,
    };
    let request = normalize(Request {
        skills: vec!["b".into(), "a".into(), "b".into()],
        harness: Some(Harness::Claude),
        passthrough: vec!["secret".into()],
        effort: Some(Effort::High),
        ..Default::default()
    });
    assert_eq!(request.skills, ["b", "a"]);
    let json = serde_json::to_value(request).unwrap();
    assert_eq!(json["effort"], "high");
    assert!(json.get("harness").is_none());
    assert!(json.get("passthrough").is_none());
}
