//! A tar archive of a few files, as `tar -x` in a Pod takes them: enough to put glave and its
//! token into a worker. ustar headers, owned by root, no timestamps.

/// One block of a tar archive.
const BLOCK: usize = 512;
/// The longest name a plain ustar header holds.
const MAX_NAME: usize = 99;

/// A file to archive.
pub(crate) struct Entry<'a> {
    pub(crate) name: &'a str,
    pub(crate) mode: u32,
    pub(crate) content: &'a [u8],
}

/// Why an archive could not be made.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TooLong(pub(crate) String);

/// The archive of `entries`, in order.
pub(crate) fn archive(entries: &[Entry<'_>]) -> Result<Vec<u8>, TooLong> {
    let mut out = Vec::new();
    for Entry {
        name,
        mode,
        content,
    } in entries
    {
        if name.len() > MAX_NAME {
            return Err(TooLong((*name).to_owned()));
        }
        out.extend_from_slice(&header(name, *mode, content.len()));
        out.extend_from_slice(content);
        let pad = (BLOCK - content.len() % BLOCK) % BLOCK;
        out.resize(out.len().saturating_add(pad), 0);
    }
    // Two empty blocks end the archive.
    out.resize(out.len().saturating_add(2 * BLOCK), 0);
    Ok(out)
}

/// An archive of just a worker's token, at `at` from `/`, put at `/` to renew it.
pub(crate) fn token_file(
    at: &str,
    token: &crate::token::Token,
) -> Result<Vec<u8>, glave_ports::computer::ComputerError> {
    let file = token.to_file();
    archive(&[Entry {
        name: at,
        mode: 0o600,
        content: file.as_bytes(),
    }])
    .map_err(|long| {
        glave_ports::computer::ComputerError::Failed(format!(
            "{} is too long a name for tar",
            long.0
        ))
    })
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

/// A regular file's header.
fn header(name: &str, mode: u32, size: usize) -> [u8; BLOCK] {
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
    put(&mut header, 156, b"0");
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
        let dir = glave_ports::testing::TempDir::new("tar").unwrap();
        let big = vec![b'x'; 1500];
        let bytes = archive(&[
            Entry {
                name: "opt/glave/run",
                mode: 0o755,
                content: b"#!/bin/sh\n",
            },
            Entry {
                name: "opt/glave/token",
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
            .arg(dir.path())
            .status()
            .unwrap();
        assert!(status.success());
        let run = dir.join("opt/glave/run");
        assert_eq!(std::fs::read(&run).unwrap(), b"#!/bin/sh\n");
        assert_eq!(
            std::fs::metadata(&run).unwrap().permissions().mode() & 0o777,
            0o755
        );
        let token = dir.join("opt/glave/token");
        assert_eq!(std::fs::read(&token).unwrap(), big);
        assert_eq!(
            std::fs::metadata(&token).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            archive(&[Entry {
                name: &"n".repeat(120),
                mode: 0o644,
                content: b"",
            }])
            .is_err()
        );
    }
}
