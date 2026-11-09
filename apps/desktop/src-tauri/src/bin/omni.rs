fn main() {
    if let Err(error) = omniterm_desktop::open_call::helper_main() {
        eprintln!("omni: {error}");
        std::process::exit(2);
    }
}
