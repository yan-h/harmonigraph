fn main() {
    #[cfg(target_os = "macos")]
    harmonigraph_plugin::editor_startup_probe();
    #[cfg(not(target_os = "macos"))]
    panic!("the native editor startup probe requires macOS");
}
