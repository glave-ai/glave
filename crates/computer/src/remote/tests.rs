use super::other_glave;

#[test]
fn another_glave_on_a_computer_is_told_to_the_admin_with_the_one_to_set() {
    let ours = env!("CARGO_PKG_VERSION");
    assert_eq!(other_glave(Some(ours)), None);
    assert_eq!(
        other_glave(Some("9.9.9")),
        Some(format!(
            "a computer runs glave 9.9.9, but this server runs glave {ours}, whose skills \
             may teach commands that one lacks: set [workers] glave to a Linux build of \
             glave {ours}; each computer takes it at its next start or wake"
        ))
    );
    let unknown = other_glave(None).unwrap_or_default();
    assert!(unknown.starts_with("a computer runs an unknown version of glave, but"));
}
