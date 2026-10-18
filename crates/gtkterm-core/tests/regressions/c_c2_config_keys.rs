//! C2: the C config code wrote `show_index` under the block-cursor key, inverted the port lock
//! and swapped rows and columns. Properties: legacy keys import into the right fields, and the
//! TOML configuration round-trips them.

use std::path::Path;

use gtkterm_core::config::{Config, DEFAULT_PROFILE, import_legacy_keyfile};
use proptest::prelude::*;

proptest! {
    #[test]
    fn c_c2_legacy_keys_land_in_their_own_fields(
        index in any::<bool>(),
        block in any::<bool>(),
        no_lock in any::<bool>(),
        rows in 10u16..60,
        columns in 81u16..300,
    ) {
        let text = format!(
            "[default]\nport=/dev/ttyUSB0\nterm_index={index}\nterm_block_cursor={block}\n\
             disable_port_lock={no_lock}\nterm_rows={rows}\nterm_columns={columns}\n"
        );
        let (cfg, warnings) = import_legacy_keyfile(&text);
        prop_assert!(warnings.is_empty(), "{:?}", warnings);
        let p = cfg.profile(DEFAULT_PROFILE).unwrap();
        prop_assert_eq!(p.terminal.show_index, index);
        prop_assert_eq!(p.terminal.block_cursor, block);
        prop_assert_eq!(p.terminal.rows, rows);
        prop_assert_eq!(p.terminal.columns, columns);
        prop_assert_eq!(p.connection.serial().unwrap().disable_port_lock, no_lock);

        let back = Config::from_toml(&cfg.to_toml().unwrap(), Path::new("config.toml")).unwrap();
        let q = back.profile(DEFAULT_PROFILE).unwrap();
        prop_assert_eq!(
            (q.terminal.show_index, q.terminal.block_cursor, q.terminal.rows, q.terminal.columns),
            (index, block, rows, columns)
        );
        prop_assert_eq!(q.connection.serial().unwrap().disable_port_lock, no_lock);
    }
}
