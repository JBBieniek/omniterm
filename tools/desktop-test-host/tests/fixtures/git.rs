//! Adversarial process fixture only; the native positive test uses real Git.
use std::io::Write;
fn main() {
    if std::env::args().any(|a| a == "--owned-child") {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    let cwd = std::env::current_dir().unwrap();
    let mode = std::fs::read_to_string(cwd.join("mode")).unwrap();
    match mode.trim() {
        "split" => {
            for b in " M é界.txt\r\nA  staged\n?? new\n".as_bytes() {
                std::io::stdout().write_all(&[*b]).unwrap();
                std::io::stdout().flush().unwrap();
                std::thread::sleep(std::time::Duration::from_millis(3));
            }
            eprintln!("fixture stderr stays separate");
        }
        "malformed" => {
            println!("UU conflict");
            eprintln!("fixture stderr survives parser rejection");
        }
        "invalid" => {
            std::io::stdout().write_all(b"?? bad\xff\n").unwrap();
        }
        "flood" => {
            for _ in 0..20000 {
                println!("?? file-{}", "x".repeat(32));
                eprintln!("stderr flood");
            }
        }
        "nonzero" => {
            println!("?? preserved\n");
            eprintln!("explicit failure");
            std::process::exit(7);
        }
        "slow" => {
            std::fs::write(cwd.join("parent.pid"), std::process::id().to_string()).unwrap();
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .arg("--owned-child")
                .spawn()
                .unwrap();
            std::fs::write(cwd.join("child.pid"), child.id().to_string()).unwrap();
            println!("?? before-cancel");
            std::io::stdout().flush().unwrap();
            std::thread::sleep(std::time::Duration::from_secs(90));
            let _ = child.wait();
        }
        _ => panic!("unknown fixture mode"),
    }
}
