//! Tool arguments from an agent never panic the MCP server's argument handling: JSON for every
//! tool's parameters, data encodings, connection specs, byte patterns and resource queries.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    gtkterm_mcp::fuzzing::tool_arguments(&String::from_utf8_lossy(data));
});
