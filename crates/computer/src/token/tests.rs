use super::*;
use std::time::UNIX_EPOCH;

use glave_ports::block_on;

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

#[test]
fn a_minted_token_opens_connections_only_until_it_expires() {
    let token = Token::mint(at(1_000), Duration::from_secs(60)).unwrap();
    assert_eq!(token.secret().len(), 64);
    assert!(token.admits(token.secret(), at(1_059)));
    assert!(!token.admits(token.secret(), at(1_060)), "expired");
    assert!(!token.admits("guess", at(1_001)));
    let other = Token::mint(at(1_000), Duration::from_secs(60)).unwrap();
    assert_ne!(token.secret(), other.secret(), "random every time");
    assert!(!format!("{token:?}").contains(token.secret()));
}

#[test]
fn tokens_survive_their_file_and_short_ones_are_refused() {
    let token = Token::mint(at(5), Duration::from_secs(10)).unwrap();
    let back = Token::from_file(&token.to_file(), at(5)).unwrap();
    assert_eq!(back, token);
    assert_eq!(back.expires(), Some(at(15)));
    let lasting = Token::from_file("0123456789abcdef\n", at(0)).unwrap();
    assert!(lasting.admits("0123456789abcdef", at(u64::from(u32::MAX))));
    assert!(Token::from_file("short\n", at(0)).is_err());
    assert!(Token::from_file("0123456789abcdef\nsoon\n", at(0)).is_err());
}

/// The harness mints a worker's token on its clock; the worker checks it on its own. Either
/// clock may be ahead, by more than the token's whole lifetime: the worker still gives it
/// its full ten minutes from when it reads it, and not a second more.
#[test]
fn clocks_that_disagree_never_shorten_or_stretch_a_token() {
    let harness_now = at(1_000_000);
    let lifetime = Duration::from_mins(10);
    let token = Token::mint(harness_now, lifetime).unwrap();
    let file = token.to_file();
    for skew in [-3_600_i64, -599, -1, 0, 1, 599, 3_600] {
        let worker_now = at(1_000_000_u64.saturating_add_signed(skew));
        let read = Token::from_file(&file, worker_now).unwrap();
        let later = |seconds: u64| worker_now + Duration::from_secs(seconds);
        assert!(read.admits(token.secret(), worker_now), "skew {skew}");
        assert!(read.admits(token.secret(), later(599)), "skew {skew}");
        assert!(!read.admits(token.secret(), later(600)), "skew {skew}");
    }
}

#[test]
fn secrets_compare_whole() {
    assert!(same("abc", "abc"));
    assert!(!same("abd", "abc"));
    assert!(!same("ab", "abc"));
}

/// A listener reading its token from a file keeps a token's expiry while the file holds it,
/// and starts a renewed one's lifetime when it first reads it.
#[test]
fn a_token_file_is_read_again_and_a_renewed_token_starts_fresh() {
    let dir = std::env::temp_dir().join(format!("glave-tokens-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("token");
    let lifetime = Duration::from_mins(10);
    let first = Token::mint(at(0), lifetime).unwrap();
    std::fs::write(&path, first.to_file()).unwrap();
    let tokens = Tokens::file(&path);
    block_on!(async {
        let read = tokens.current(at(100)).await.unwrap();
        assert_eq!(read.expires(), Some(at(700)));
        let again = tokens.current(at(650)).await.unwrap();
        assert_eq!(
            again.expires(),
            Some(at(700)),
            "reading again never extends it"
        );
        assert!(!again.admits(first.secret(), at(700)));

        let renewed = Token::mint(at(5_000), lifetime).unwrap();
        std::fs::write(&path, renewed.to_file()).unwrap();
        let read = tokens.current(at(800)).await.unwrap();
        assert!(read.admits(renewed.secret(), at(1_399)));
        assert!(
            !read.admits(first.secret(), at(800)),
            "the old secret opens nothing"
        );

        let fixed = Tokens::from(first.clone());
        assert_eq!(fixed.current(at(1)).await.unwrap(), first);
        std::fs::remove_file(&path).unwrap();
        assert!(
            tokens.current(at(900)).await.is_err(),
            "no file, no connection"
        );
    });
}
