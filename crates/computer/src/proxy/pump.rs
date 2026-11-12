//! Bytes both ways between a command's connection and the host's, through one buffer each way,
//! until both sides are done or nothing has moved either way for the idle limit.

use std::io;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;

/// Each way's buffer.
const BUFFER: usize = 16 * 1024;

/// Copies `a` to `b` and `b` to `a` until each has ended (a side that ends has its other's
/// writing shut), or until `idle` passes with nothing read from either.
pub(crate) async fn pump<A, B>(a: &mut A, b: &mut B, idle: Duration) -> io::Result<()>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let mut from_a = vec![0_u8; BUFFER];
    let mut from_b = vec![0_u8; BUFFER];
    let (mut a_open, mut b_open) = (true, true);
    while a_open || b_open {
        // Both reads are cancel-safe: the one not chosen has read nothing.
        let read = timeout(idle, async {
            tokio::select! {
                read = a.read(&mut from_a), if a_open => (true, read),
                read = b.read(&mut from_b), if b_open => (false, read),
            }
        })
        .await;
        let Ok((from_a_side, read)) = read else {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("nothing moved for {}", glave_ports::spoken(idle)),
            ));
        };
        let n = read?;
        if from_a_side {
            carry(from_a.get(..n), b, &mut a_open, idle).await?;
        } else {
            carry(from_b.get(..n), a, &mut b_open, idle).await?;
        }
    }
    Ok(())
}

/// Writes what one side read to the other, or, where it read nothing, shuts the other's
/// writing and marks the side ended.
async fn carry<W: AsyncWrite + Unpin>(
    read: Option<&[u8]>,
    sink: &mut W,
    open: &mut bool,
    idle: Duration,
) -> io::Result<()> {
    match read {
        Some([]) | None => {
            *open = false;
            timeout(idle, sink.shutdown()).await.unwrap_or(Ok(()))
        }
        Some(bytes) => timeout(idle, sink.write_all(bytes)).await.map_err(|_| {
            io::Error::new(io::ErrorKind::TimedOut, "the other side stopped reading")
        })?,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_go_both_ways_until_both_sides_end() {
        glave_ports::block_on!(async {
            let (mut near, mut a) = tokio::io::duplex(64);
            let (mut b, mut far) = tokio::io::duplex(64);
            let pumping =
                tokio::spawn(async move { pump(&mut a, &mut b, Duration::from_secs(60)).await });
            near.write_all(b"ping").await.unwrap();
            let mut got = [0_u8; 4];
            far.read_exact(&mut got).await.unwrap();
            assert_eq!(&got, b"ping");
            far.write_all(b"pong").await.unwrap();
            near.read_exact(&mut got).await.unwrap();
            assert_eq!(&got, b"pong");
            near.shutdown().await.unwrap();
            assert_eq!(far.read(&mut got).await.unwrap(), 0, "the end passes on");
            far.write_all(b"late").await.unwrap();
            near.read_exact(&mut got).await.unwrap();
            assert_eq!(&got, b"late", "the other way stays open");
            drop(far);
            assert_eq!(near.read(&mut got).await.unwrap(), 0);
            pumping.await.unwrap().unwrap();
        });
    }

    #[test]
    fn a_tunnel_with_nothing_moving_closes() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap();
        runtime.block_on(async {
            let (_near, mut a) = tokio::io::duplex(64);
            let (mut b, _far) = tokio::io::duplex(64);
            let ended = pump(&mut a, &mut b, Duration::from_secs(300)).await;
            let error = ended.unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            assert_eq!(error.to_string(), "nothing moved for 5 minutes");
        });
    }
}
