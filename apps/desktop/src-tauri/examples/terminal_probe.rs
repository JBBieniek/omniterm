use omniterm_desktop::terminal::{ScreenSnapshot, TerminalHost};
use std::time::{Duration, Instant};

fn text(screen: &ScreenSnapshot) -> String {
    screen
        .lines
        .iter()
        .map(|row| row.iter().map(|run| run.text.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
fn wait(host: &TerminalHost, id: u64, needle: &str) -> ScreenSnapshot {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let screen = host.screen(id, None).unwrap().unwrap();
        if text(&screen).contains(needle) {
            return screen;
        }
        if Instant::now() > deadline {
            panic!("Terminal did not show {needle:?}: {}", text(&screen));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn command(host: &TerminalHost, id: u64, text: &str) {
    host.input(id, text, false).unwrap();
    host.key(id, "Enter", false, false, false).unwrap();
}

fn main() {
    let cwd = std::env::args_os()
        .nth(1)
        .expect("Usage: terminal_probe TEST_OWNED_DIRECTORY");
    let directory = std::path::PathBuf::from(cwd);
    // A test-only sentinel must not cross the production child environment filter.
    std::env::set_var("OMNITERM_PROBE_SECRET", "must-not-inherit");
    let host = TerminalHost::new(directory.clone());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let id = host.start().unwrap();
        command(&host, id, "Write-Output ('OMNI_' + 'READY')");
        wait(&host, id, "OMNI_READY");
        command(
            &host,
            id,
            "Write-Output ([string][char]0xe9 + [char]0x754c)",
        );
        wait(&host, id, "é界");
        command(
            &host,
            id,
            "if (Test-Path Env:OMNITERM_PROBE_SECRET) { 'ENV_LEAK' } else { 'ENV_' + 'FILTERED' }",
        );
        wait(&host, id, "ENV_FILTERED");
        host.resize(id, 18, 72).unwrap();
        command(
            &host,
            id,
            "Write-Output ('SIZE=' + [Console]::WindowWidth + 'x' + [Console]::WindowHeight)",
        );
        wait(&host, id, "SIZE=72x18");
        command(&host, id, "Start-Sleep -Seconds 20");
        std::thread::sleep(Duration::from_millis(200));
        host.key(id, "c", true, false, false).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        command(&host, id, "Write-Output ('AFTER_' + 'INTERRUPT')");
        wait(&host, id, "AFTER_INTERRUPT");
        command(&host,id,"1..8000 | ForEach-Object { Write-Output ('x' * 64) }; Write-Output ('FLOOD_' + 'DONE')");
        let screen = wait(&host, id, "FLOOD_DONE");
        assert!(
            screen.evicted_bytes > 0,
            "Raw quota must become visibly evicted"
        );
        command(&host, id, "exit 7");
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let screen = host.screen(id, None).unwrap().unwrap();
            if screen.state == "exited" {
                assert_eq!(serde_json::to_value(&screen).unwrap()["exit_code"], 7);
                break;
            }
            assert!(Instant::now() < deadline, "Exit/EOF not confirmed");
            std::thread::sleep(Duration::from_millis(10));
        }
        let second = host.start().unwrap();
        assert_ne!(second, id);
        command(&host, second, "Write-Output ('SECOND_' + 'READY')");
        wait(&host, second, "SECOND_READY");
        command(&host, second, "$p=Start-Process powershell.exe -ArgumentList '-NoLogo','-NoProfile','-Command','Start-Sleep 120' -NoNewWindow -PassThru; $p.Id | Set-Content child.pid; Write-Output ('CHILD_'+'READY')");
        wait(&host, second, "CHILD_READY");
        #[cfg(windows)]
        let descendant = {
            use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};
            let pid = std::fs::read_to_string(directory.join("child.pid"))
                .unwrap()
                .trim()
                .parse::<u32>()
                .unwrap();
            unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid).unwrap() }
        };
        host.terminate(second).unwrap();
        host.start().unwrap();
        host.start().unwrap();
        assert!(
            host.start().is_err(),
            "Four retained records must cap spawning"
        );
        host.shutdown().unwrap();
        assert!(!host.has_live());
        #[cfg(windows)]
        unsafe {
            use windows::Win32::{
                Foundation::{CloseHandle, WAIT_OBJECT_0},
                System::Threading::WaitForSingleObject,
            };
            assert_eq!(
                WaitForSingleObject(descendant, 4000),
                WAIT_OBJECT_0,
                "Owned descendant survived termination"
            );
            CloseHandle(descendant).unwrap();
        }
        println!("PASS: native PowerShell PTY, Unicode, filtered environment, console resize, Ctrl-C, bounded flood, exit/EOF, owned descendant termination and shutdown");
    }));
    let cleanup = host.shutdown();
    assert!(cleanup.is_ok(), "Cleanup failed: {cleanup:?}");
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
