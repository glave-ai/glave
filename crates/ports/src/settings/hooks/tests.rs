use super::*;

fn hook(point: HookPoint, matcher: Option<&str>) -> Hook {
    Hook {
        point,
        command: "true".into(),
        matcher: matcher.map(str::to_owned),
        timeout_secs: None,
    }
}

#[test]
fn a_matcher_matches_the_whole_tool_name() {
    let hooks = Hooks::new(vec![
        hook(HookPoint::PreToolUse, Some("edit|write")),
        hook(HookPoint::PreToolUse, Some("*")),
        hook(HookPoint::PostToolUse, None),
    ])
    .unwrap();
    assert_eq!(hooks.at(HookPoint::PreToolUse, Some("edit")).len(), 2);
    assert_eq!(hooks.at(HookPoint::PreToolUse, Some("bash")).len(), 1);
    assert_eq!(hooks.at(HookPoint::PreToolUse, Some("edits")).len(), 1);
    assert_eq!(hooks.at(HookPoint::PostToolUse, Some("bash")).len(), 1);
    assert_eq!(hooks.at(HookPoint::Stop, None).len(), 0);
}

#[test]
fn bad_hooks_are_refused_naming_the_problem() {
    let bad = Hooks::new(vec![hook(HookPoint::PreToolUse, Some("("))]).unwrap_err();
    assert!(
        bad.to_string().contains("not a regular expression"),
        "{bad}"
    );
    let stray = Hooks::new(vec![hook(HookPoint::Stop, Some("bash"))]).unwrap_err();
    assert!(
        stray.to_string().contains("do not run for a tool"),
        "{stray}"
    );
    let mut empty = hook(HookPoint::Stop, None);
    empty.command = "  ".into();
    assert!(Hooks::new(vec![empty]).is_err());
}

#[test]
fn a_bad_hook_is_named_by_where_it_is_never_by_its_command() {
    let secret = crate::testing::any_secret();
    let with_token = |point, matcher| Hook {
        command: format!("curl -H 'Authorization: Bearer {secret}' https://hooks.example"),
        ..hook(point, matcher)
    };
    for bad in [
        vec![
            hook(HookPoint::PreToolUse, None),
            with_token(HookPoint::PreToolUse, Some("(")),
        ],
        vec![
            hook(HookPoint::Stop, None),
            with_token(HookPoint::Stop, Some("bash")),
        ],
    ] {
        let refused = Hooks::new(bad).unwrap_err().to_string();
        assert!(!refused.contains(&secret), "the command said");
        assert!(refused.contains("hook number 2"), "{refused}");
    }
}
