//! glave's memory passes the memory port's scope suite on a sealed store: a project's memory
//! stays in its project, a person's own with them and in their organization, and what someone
//! may not reach is answered as missing.

use glave_ports::conformance::{MemoryPlaces, memory_scopes};

use crate::support::*;

#[test]
fn glaves_memory_passes_the_scope_suite() {
    block_on!(async {
        let kit = Kit::new();
        let memory = kit.memory(rules(), 1).await;
        let places = MemoryPlaces {
            here: shared(),
            next_door: scope(1, 5),
            abroad: scope(9, 6),
            person: person(),
            stranger: glave_event::PrincipalId::try_from(uuid(52)).unwrap(),
        };
        assert_eq!(memory_scopes(memory.as_ref(), places).await, Ok(()));
    });
}
