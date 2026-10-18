//! Named regression tests for the C defects of ROADMAP §8 that are not covered by a unit or
//! feature test elsewhere. `README.md` maps every defect ID to its guard.
#![allow(clippy::unwrap_used)]

mod c_b4_drop_order;
mod c_c2_config_keys;
mod c_p2_modem_lines;
mod c_p4_partial_writes;
