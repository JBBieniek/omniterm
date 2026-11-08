#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(windows)]
mod automation;

#[cfg(windows)]
fn main() {
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--automation"] {
        eprintln!("test host requires explicit --automation launch");
        std::process::exit(2);
    }
    if let Err(error) = automation::prepare_pipes() {
        eprintln!("{error}");
        std::process::exit(2);
    }
    // This is the production app's construction path, content, commands and ACL.
    let app = omniterm_desktop::application().expect("failed to build the test app");
    automation::start(app.handle().clone());
    omniterm_desktop::run_app(app);
}

#[cfg(not(windows))]
fn main() {
    eprintln!("native test host is currently Windows-only");
    std::process::exit(2);
}
