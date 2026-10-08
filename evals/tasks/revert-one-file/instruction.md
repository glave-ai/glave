The last commit broke `round_to_dollars` in src/lib.rs. Undo that commit's change to
src/lib.rs only, keeping everything else it did, and commit the result with the message
`fix: restore rounding`.
