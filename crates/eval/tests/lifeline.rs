//! The model's commands are tied to the eval by glave's lifeline (#558): the eval's own program
//! answers as the lifeline's helper, which is how its command line starts it.

#[test]
fn the_eval_s_program_is_its_own_lifeline_helper() {
    let helper = env!("CARGO_BIN_EXE_glave-eval");
    let tied = glave_ports::block_on!(glave_process::Processes::unrecorded().with_lifeline(helper));
    assert!(tied.is_ok(), "{:?}", tied.err());
}
