//! glave's own memory server passes the conformance suite every memory server must, through MCP
//! with who and where in `_meta`, on a sealed store.

#[cfg(test)]
#[path = "support/kit.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod conformance {
    use glave_harness::MemoryContext;
    use glave_memory::conformance::{Fixture, memory};
    use glave_ports::Scope;

    use super::support::*;

    fn on(scope: Scope, thread: u128) -> MemoryContext {
        MemoryContext {
            person: Some(person()),
            agent: Some(agent()),
            scope,
            thread: glave_event::ThreadId::try_from(uuid(thread)).unwrap(),
            seq: None,
            untrusted_read: false,
        }
    }

    #[test]
    fn glaves_memory_server_conforms() {
        block_on(async {
            let kit = Kit::new();
            let server = kit.memory(rules(), 1).await;
            let fixture = Fixture {
                here: on(shared(), 60),
                elsewhere: on(scope(1, 5), 61),
                isolated: on(isolated(), 62),
            };
            assert_eq!(memory(server.as_ref(), &fixture).await, Ok(()));
        });
    }
}
