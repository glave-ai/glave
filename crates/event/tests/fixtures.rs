//! Every event version this crate can read keeps a saved example under `fixtures/`. These
//! tests prove each one still decodes, that the latest versions still encode to exactly the
//! saved bytes, and that no saved example is missing or orphaned.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use glave_event::{Body, KNOWN_VERSIONS, RawEvent, decode};

type TestResult = Result<(), Box<dyn Error>>;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn fixture_path(kind: &str, version: u32) -> PathBuf {
    fixtures_dir().join(kind).join(format!("v{version}.json"))
}

fn latest_versions() -> BTreeMap<&'static str, u32> {
    let mut latest = BTreeMap::new();
    for (kind, version) in KNOWN_VERSIONS {
        let entry = latest.entry(*kind).or_insert(*version);
        *entry = (*entry).max(*version);
    }
    latest
}

#[test]
fn every_known_version_has_a_fixture_that_decodes() -> TestResult {
    for (kind, version) in KNOWN_VERSIONS {
        let path = fixture_path(kind, *version);
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let raw = RawEvent::from_json(text.trim_end())?;
        assert_eq!(raw.kind.as_str(), *kind, "{}", path.display());
        assert_eq!(raw.v.get(), *version, "{}", path.display());
        let event = decode(raw)?;
        assert!(
            !matches!(event.body, Body::Unknown(_)),
            "{} decoded as unknown",
            path.display()
        );
    }
    Ok(())
}

#[test]
fn latest_versions_encode_to_exactly_the_saved_bytes() -> TestResult {
    for (kind, version) in latest_versions() {
        let path = fixture_path(kind, version);
        let text = fs::read_to_string(&path)?;
        let saved = text.trim_end();
        let encoded = decode(RawEvent::from_json(saved)?)?.to_raw()?.to_json()?;
        assert_eq!(
            encoded,
            saved,
            "{} no longer encodes the same way",
            path.display()
        );
    }
    Ok(())
}

#[test]
fn every_fixture_is_a_known_version() -> TestResult {
    let known: BTreeSet<(String, u32)> = KNOWN_VERSIONS
        .iter()
        .map(|(kind, version)| ((*kind).to_owned(), *version))
        .collect();
    for type_dir in fs::read_dir(fixtures_dir())? {
        let type_dir = type_dir?;
        let kind = type_dir.file_name().to_string_lossy().into_owned();
        for file in fs::read_dir(type_dir.path())? {
            let name = file?.file_name().to_string_lossy().into_owned();
            let version: u32 = name
                .strip_prefix('v')
                .and_then(|rest| rest.strip_suffix(".json"))
                .ok_or_else(|| format!("{kind}/{name} is not named v<N>.json"))?
                .parse()?;
            assert!(
                known.contains(&(kind.clone(), version)),
                "fixture {kind}/{name} is not listed in KNOWN_VERSIONS"
            );
        }
    }
    Ok(())
}
