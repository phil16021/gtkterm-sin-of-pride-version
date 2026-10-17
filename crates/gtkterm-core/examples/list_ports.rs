//! Print the serial ports found on this machine: `cargo run -p gtkterm-core --example list_ports`.
fn main() {
    for p in gtkterm_core::ports::list_ports() {
        println!("{:<12?} {}", p.class, p.label());
    }
}
