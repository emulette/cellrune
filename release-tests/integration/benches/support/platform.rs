#[cfg(target_os = "linux")]
use std::fs;
use std::process::Command;

#[cfg(target_os = "linux")]
pub(super) fn current_rss_bytes(pid: u32) -> usize {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).expect("read child status");
    let kibibytes = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|line| line.split_whitespace().next())
        .expect("VmRSS value")
        .parse::<usize>()
        .expect("numeric VmRSS");
    kibibytes * 1_024
}

#[cfg(target_os = "windows")]
pub(super) fn current_rss_bytes(pid: u32) -> usize {
    let script = format!(
        "$process = Get-Process -Id {pid} -ErrorAction Stop; \
         [Console]::Out.Write($process.WorkingSet64)"
    );
    command_output(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
    )
    .parse()
    .expect("numeric Windows working set")
}

#[cfg(all(unix, not(target_os = "linux")))]
pub(super) fn current_rss_bytes(pid: u32) -> usize {
    command_output("ps", &["-o", "rss=", "-p", &pid.to_string()])
        .parse::<usize>()
        .expect("numeric ps RSS")
        * 1_024
}

#[cfg(not(any(unix, target_os = "windows")))]
pub(super) fn current_rss_bytes(_pid: u32) -> usize {
    panic!("RSS measurement is unsupported on this target")
}

#[cfg(target_os = "macos")]
pub(super) fn machine_identity() -> String {
    command_output("sysctl", &["-n", "hw.model"])
}

#[cfg(target_os = "linux")]
pub(super) fn machine_identity() -> String {
    let machine = command_output("uname", &["-m"]);
    let cpu = fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.strip_prefix("model name")
                    .and_then(|value| value.split_once(':').map(|(_, model)| model.trim()))
                    .map(str::to_owned)
            })
        })
        .unwrap_or_else(|| "unknown-cpu".to_owned());
    format!("{machine}:{cpu}")
}

#[cfg(target_os = "windows")]
pub(super) fn machine_identity() -> String {
    let processor =
        std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "unknown-cpu".to_owned());
    format!("{}:{processor}", std::env::consts::ARCH)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) fn machine_identity() -> String {
    command_output("uname", &["-m"])
}

pub(super) fn command_output(program: &str, arguments: &[&str]) -> String {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("run metadata command {program}: {error}"));
    assert!(output.status.success(), "{program} failed");
    String::from_utf8(output.stdout)
        .expect("metadata UTF-8")
        .trim()
        .to_owned()
}
