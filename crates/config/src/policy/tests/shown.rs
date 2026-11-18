//! Settings that carry a secret in a URL (a password, a token in a query) never say it: not in
//! the errors that refuse them, not in `policy explain`, not in `Debug` (invariant 8).

use glave_ports::testing::any_secret;

use super::reading::{load_all, scratch};
use super::*;
use crate::policy::files::{ServerFile, StoreFile, TelemetryFile};

#[test]
fn a_url_refused_is_said_without_what_it_carries() {
    let secret = any_secret();
    let path = Path::new("/home/ada/.glave/config.toml");
    for (key, text) in [
        (
            "[telemetry] otlp",
            format!("[telemetry]\notlp = \"collector.example.com/v1?token={secret}\"\n"),
        ),
        (
            "[server] url",
            format!("[server]\nurl = \"http://me:{secret}@glave.example.com\"\n"),
        ),
        (
            "[store] bucket",
            format!("[store]\nbucket = \"gs://blobs?key={secret}\"\n"),
        ),
        (
            "[store] url",
            format!("[store]\nurl = \"mysql://glave:{secret}@db/glave?key={secret}\"\n"),
        ),
    ] {
        let refused = files::config(&text, path).unwrap_err().to_string();
        assert!(!refused.contains(&secret), "{key}: what it carries said");
        assert!(refused.contains(key), "{refused}");
    }
}

#[test]
fn explain_shows_where_each_url_points_never_what_it_carries() {
    let dir = scratch("shown");
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let secret = any_secret();
    let text = format!(
        "[telemetry]\notlp = \"https://collector.example.com/?token={secret}\"\n\
         [server]\nurl = \"https://glave.example.com/?token={secret}\"\n\
         [store]\nurl = \"postgres://glave:{secret}@db.example.com/glave?sslmode=require\"\n\
         bucket = \"s3://blobs?key={secret}\"\n"
    );
    std::fs::write(home.join("config.toml"), text).unwrap();
    let mut locations = Locations::here(&home, &dir);
    locations.policies = vec![];
    let loaded = load_all(&locations, None).unwrap();
    let (said, _) = explain(&loaded, None);
    assert!(!said.contains(&secret), "what a URL carries said");
    for host in [
        "collector.example.com",
        "glave.example.com",
        "db.example.com",
    ] {
        assert!(said.contains(host), "{said}");
    }
}

#[test]
fn a_setting_s_debug_never_shows_what_its_url_carries() {
    let secret = any_secret();
    let store = StoreFile {
        url: Some(format!("postgres://glave:{secret}@db/glave")),
        bucket: Some(format!("s3://blobs?key={secret}")),
    };
    let telemetry = TelemetryFile {
        otlp: format!("https://collector.example.com/?token={secret}"),
    };
    let server = ServerFile {
        url: format!("https://{secret}@glave.example.com"),
    };
    for debug in [
        format!("{store:?}"),
        format!("{telemetry:?}"),
        format!("{server:?}"),
    ] {
        assert!(!debug.contains(&secret), "what a URL carries in Debug");
        assert!(
            debug.contains("example.com") || debug.contains("db"),
            "{debug}"
        );
    }
}
