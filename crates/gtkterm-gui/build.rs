fn main() {
    #[cfg(feature = "gui")]
    glib_build_tools::compile_resources(
        &["../../data"],
        "../../data/resources.gresource.xml",
        "gtkterm.gresource",
    );
}
