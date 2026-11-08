use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::*;

#[test]
fn lsof_is_read_by_group_keeping_what_loopback_reaches_with_its_family() {
    // As macOS 26's lsof prints it for a group listening at each kind of address, then a
    // second group listening on a private address only.
    let printed = "p83733\ng83730\nf3\ntIPv4\nn127.0.0.1:61946\nf4\ntIPv6\nn[::1]:61947\nf5\n\
                   tIPv4\nn*:61948\nf6\ntIPv6\nn*:61949\np900\ng900\nf7\ntIPv4\n\
                   n192.168.1.5:8000\n";
    let found = in_lsof(printed);
    assert_eq!(found.len(), 1, "{found:?}");
    let at: Vec<(u16, IpAddr)> = found[&83730].iter().copied().collect();
    assert_eq!(
        at,
        [
            (61946, IpAddr::V4(Ipv4Addr::LOCALHOST)),
            (61947, IpAddr::V6(Ipv6Addr::LOCALHOST)),
            (61948, IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            (61949, IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
        ]
    );
    assert!(in_lsof("").is_empty());
}

#[test]
fn a_stat_line_names_the_parent_and_the_group_whatever_the_name_holds() {
    let stat = "4242 (node (dev) 1 2) S 4200 4100 4100 0 -1 4194560 0 0";
    assert_eq!(
        in_stat(stat),
        Some(Process {
            parent: 4200,
            group: 4100
        })
    );
    assert_eq!(in_stat("4242 (node"), None);
}

#[test]
fn a_command_s_processes_are_its_group_and_what_they_started_below_it() {
    let processes: HashMap<u32, Process> = [
        // The command's own shell, its group's leader.
        (
            100,
            Process {
                parent: 1,
                group: 100,
            },
        ),
        // The sandbox's own session, below it.
        (
            101,
            Process {
                parent: 100,
                group: 101,
            },
        ),
        (
            102,
            Process {
                parent: 101,
                group: 101,
            },
        ),
        // Another command, and something no command started.
        (
            200,
            Process {
                parent: 1,
                group: 200,
            },
        ),
        (
            300,
            Process {
                parent: 1,
                group: 300,
            },
        ),
    ]
    .into_iter()
    .collect();
    let owned = owners(&processes, &[100, 200]);
    let mut owned: Vec<(u32, u32)> = owned.into_iter().collect();
    owned.sort_unstable();
    assert_eq!(owned, [(100, 100), (101, 100), (102, 100), (200, 200)]);
}

#[test]
fn a_tcp_table_gives_the_sockets_that_listen_where_loopback_reaches_them() {
    // `/proc/net/tcp` on x86-64: 127.0.0.1:8080 and 0.0.0.0:3000 listening, 10.0.0.5:22
    // listening on a private address only, and a connection that does not listen.
    let tcp = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 11111 1 0\n\
   1: 00000000:0BB8 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 22222 1 0\n\
   2: 0500000A:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 33333 1 0\n\
   3: 0100007F:1F90 0100007F:D431 01 00000000:00000000 00:00000000 00000000  1000        0 44444 1 0\n";
    if cfg!(target_endian = "little") {
        assert_eq!(
            in_table(tcp),
            [
                (11111, "127.0.0.1:8080".parse().unwrap()),
                (22222, "0.0.0.0:3000".parse().unwrap())
            ]
        );
    }
    // `tcp6`: [::1]:5173 and [::]:4000.
    let tcp6 = "  sl  local_address                         remote_address                        st\n\
   0: 00000000000000000000000001000000:1435 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 55555 1 0\n\
   1: 00000000000000000000000000000000:0FA0 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 66666 1 0\n";
    if cfg!(target_endian = "little") {
        assert_eq!(
            in_table(tcp6),
            [
                (55555, "[::1]:5173".parse().unwrap()),
                (66666, "[::]:4000".parse().unwrap())
            ]
        );
    }
}

#[test]
fn addresses_read_as_the_kernel_writes_them() {
    let loopback = u32::from_ne_bytes([127, 0, 0, 1]);
    assert_eq!(
        in_hex(&format!("{loopback:08X}")),
        Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
    );
    let words: Vec<String> = Ipv6Addr::LOCALHOST
        .octets()
        .chunks(4)
        .map(|word| u32::from_ne_bytes([word[0], word[1], word[2], word[3]]))
        .map(|word| format!("{word:08X}"))
        .collect();
    let words = words.concat();
    assert_eq!(in_hex(&words), Some(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    assert_eq!(in_hex("0100"), None);
    assert_eq!(in_hex("XYZW0000"), None);
}

#[test]
fn a_socket_s_link_names_its_inode() {
    assert_eq!(socket_inode("socket:[12345]"), Some(12345));
    assert_eq!(socket_inode("pipe:[12345]"), None);
    assert_eq!(socket_inode("/dev/null"), None);
}
