use glave_event::{HookPoint, HookResult};

use super::run::decided_for_tests as decided;

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
