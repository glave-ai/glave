//! Which glave a computer across a connection runs, beside this server's.

/// What to tell the admin when a computer whose glave reports `theirs` runs another version
/// than this glave: nothing for the same version, compared alone, never the commit, so a
/// developer's own Linux build of the same version serves.
pub(super) fn other_glave(theirs: Option<&str>) -> Option<String> {
    let ours = env!("CARGO_PKG_VERSION");
    if theirs == Some(ours) {
        return None;
    }
    let theirs = theirs.map_or_else(
        || "an unknown version of glave".to_owned(),
        |version| format!("glave {version}"),
    );
    Some(format!(
        "a computer runs {theirs}, but this server runs glave {ours}, whose skills may teach \
         commands that one lacks: set [workers] glave to a Linux build of glave {ours}; each \
         computer takes it at its next start or wake"
    ))
}
