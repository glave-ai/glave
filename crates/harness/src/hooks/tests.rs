use super::run::decided_for_tests as decided;
use super::*;
use glave_event::HookResult;

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
fn json_on_exit_zero_can_only_block() {
    let pre = HookPoint::PreToolUse;
    assert_eq!(decided(pre, "all good").0, HookResult::Passed);
    assert_eq!(decided(pre, "").0, HookResult::Passed);
    let block = decided(pre, r#"{"decision":"block","reason":"not here"}"#);
    assert_eq!(block, (HookResult::Blocked, Some("not here".into())));
    let deny =
        r#"{"hookSpecificOutput":{"permissionDecision":"deny","permissionDecisionReason":"no"}}"#;
    assert_eq!(decided(pre, deny), (HookResult::Blocked, Some("no".into())));
    // Allowing never skips an approval: it is read as passing, nothing more.
    let allow = r#"{"hookSpecificOutput":{"permissionDecision":"allow"}}"#;
    assert_eq!(decided(pre, allow), (HookResult::Passed, None));
    // A deny is a PreToolUse answer only.
    assert_eq!(decided(HookPoint::PostToolUse, deny).0, HookResult::Passed);
    // A Stop hook cannot keep a turn going.
    let stop = decided(HookPoint::Stop, r#"{"decision":"block","reason":"more"}"#);
    assert_eq!(stop.0, HookResult::Failed);
}
