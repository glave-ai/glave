//! A tar archive of a few folders and files, as Docker's archive endpoint takes them: enough to
//! put glave and its TLS identity into a worker. ustar headers, owned by root, no timestamps.

/// One block of a tar archive.
const BLOCK: usize = 512;
/// The longest name a plain ustar header holds.
const MAX_NAME: usize = 99;

/// An entry to archive.
pub(crate) enum Entry<'a> {
    Folder {
        name: &'a str,
        mode: u32,
    },
    File {
        name: &'a str,
        mode: u32,
        content: &'a [u8],
    },
}

/// Why an archive could not be made.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TooLong(pub(crate) String);

/// The archive of `entries`, in order.
pub(crate) fn archive(entries: &[Entry<'_>]) -> Result<Vec<u8>, TooLong> {
    let mut out = Vec::new();
    for entry in entries {
        let (name, mode, kind, content): (String, u32, u8, &[u8]) = match entry {
            Entry::Folder { name, mode } => {
                (format!("{}/", name.trim_end_matches('/')), *mode, b'5', &[])
            }
            Entry::File {
                name,
                mode,
                content,
            } => ((*name).to_owned(), *mode, b'0', content),
        };
        if name.len() > MAX_NAME {
            return Err(TooLong(name));
        }
        out.extend_from_slice(&header(&name, mode, kind, content.len()));
        out.extend_from_slice(content);
        let pad = (BLOCK - content.len() % BLOCK) % BLOCK;
        out.resize(out.len().saturating_add(pad), 0);
    }
    // Two empty blocks end the archive.
    out.resize(out.len().saturating_add(2 * BLOCK), 0);
    Ok(out)
}

/// `bytes` written into `header` at `offset`.
fn put(header: &mut [u8; BLOCK], offset: usize, bytes: &[u8]) {
    for (slot, byte) in header.iter_mut().skip(offset).zip(bytes) {
        *slot = *byte;
    }
}

/// `value` in octal, `width` digits and a NUL.
fn octal(value: u64, width: usize) -> Vec<u8> {
    let mut text = format!("{value:0width$o}").into_bytes();
    text.push(0);
    text
}

fn header(name: &str, mode: u32, kind: u8, size: usize) -> [u8; BLOCK] {
    let mut header = [0u8; BLOCK];
    put(&mut header, 0, name.as_bytes());
    put(&mut header, 100, &octal(u64::from(mode), 7));
    put(&mut header, 108, &octal(0, 7));
    put(&mut header, 116, &octal(0, 7));
    put(
        &mut header,
        124,
        &octal(u64::try_from(size).unwrap_or(u64::MAX), 11),
    );
    put(&mut header, 136, &octal(0, 11));
    put(&mut header, 148, b"        ");
    put(&mut header, 156, &[kind]);
    put(&mut header, 257, b"ustar\x0000");
    put(&mut header, 265, b"root");
    put(&mut header, 297, b"root");
    let sum: u64 = header.iter().map(|byte| u64::from(*byte)).sum();
    let mut checksum = format!("{sum:06o}").into_bytes();
    checksum.extend_from_slice(&[0, b' ']);
    put(&mut header, 148, &checksum);
    header
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    #[test]
    fn the_system_tar_reads_back_what_was_archived() {
        let dir = std::env::temp_dir().join(format!("glave-tar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let big = vec![b'x'; 1500];
        let bytes = archive(&[
            Entry::Folder {
                name: "opt/glave",
                mode: 0o755,
            },
            Entry::File {
                name: "opt/glave/run",
                mode: 0o755,
                content: b"#!/bin/sh\n",
            },
            Entry::File {
                name: "opt/glave/key.pem",
                mode: 0o600,
                content: &big,
            },
        ])
        .unwrap();
        assert_eq!(bytes.len() % BLOCK, 0);
        let file = dir.join("a.tar");
        std::fs::write(&file, &bytes).unwrap();
        let status = std::process::Command::new("tar")
            .arg("-xf")
            .arg(&file)
            .arg("-C")
            .arg(&dir)
            .status()
            .unwrap();
        assert!(status.success());
        let run = dir.join("opt/glave/run");
        assert_eq!(std::fs::read(&run).unwrap(), b"#!/bin/sh\n");
        assert_eq!(
            std::fs::metadata(&run).unwrap().permissions().mode() & 0o777,
            0o755
        );
        let key = dir.join("opt/glave/key.pem");
        assert_eq!(std::fs::read(&key).unwrap(), big);
        assert_eq!(
            std::fs::metadata(&key).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            archive(&[Entry::Folder {
                name: &"n".repeat(120),
                mode: 0o755
            }])
            .is_err()
        );
    }
}
