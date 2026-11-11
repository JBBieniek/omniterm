//! Real argv/stdin/descendant fixture; no shell or runtime dependency.
use std::io::{BufRead, Write};
fn hex(s: &str) -> String { s.as_bytes().iter().map(|b|format!("{b:02x}")).collect() }
fn main() {
    let args=std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|a|a=="--sleep") { std::thread::sleep(std::time::Duration::from_secs(120)); return; }
    if args.first().is_some_and(|a|a=="--exit") { println!("FAST_FINAL"); std::process::exit(42); }
    std::fs::write("fixture.pid",std::process::id().to_string()).unwrap();
    std::fs::write("argv.hex",args.iter().map(|s|hex(s)+"\n").collect::<String>()).unwrap();
    std::fs::write("cwd.txt",std::env::current_dir().unwrap().to_str().unwrap()).unwrap();
    assert!(std::env::var_os("OMNITERM_TEST_PARENT").is_none());
    println!("FIXTURE_READY");std::io::stdout().flush().unwrap();
    for line in std::io::stdin().lock().lines() {
        let line=line.unwrap();let line=line.trim();
        if let Some(n)=line.strip_prefix("PING-") { println!("REPLY-{n}"); }
        else if line=="OPEN" {
            let result=std::process::Command::new("omni.exe").args(["open","note.txt"]).status().unwrap();
            println!("HELPER_RESULT={}",result.code().unwrap_or(-1));
        } else if line=="LAUNCH" {
            let input=std::fs::File::open("next.json").unwrap();
            let result=std::process::Command::new("omni.exe").args(["terminal","--json"]).stdin(input).status().unwrap();
            println!("LAUNCH_RESULT={}",result.code().unwrap_or(-1));
        } else if line=="SPAWN" {
            let child=std::process::Command::new(std::env::current_exe().unwrap()).arg("--sleep").spawn().unwrap();
            std::fs::write("descendant.pid",child.id().to_string()).unwrap();
            println!("DESCENDANT_READY");
        } else if line=="CLI" {
            let result=std::process::Command::new("omni.exe").args(["terminal","--"]).arg(std::env::current_exe().unwrap()).arg("--exit").status().unwrap();
            println!("CLI_RESULT={}",result.code().unwrap_or(-1));
        } else if line=="EXIT7" { println!("FIXTURE_FINAL");std::io::stdout().flush().unwrap();std::process::exit(7); }
        std::io::stdout().flush().unwrap();
    }
}
