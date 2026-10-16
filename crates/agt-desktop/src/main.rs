//! AGT Studio: the agt console in a desktop window.
//!
//! The same executable is also the agt CLI that console operations run
//! (`agt-studio --agt-cli <agt arguments>`), so the app ships as one file.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod notify;

use std::ffi::OsString;
use std::process::ExitCode;

/// First argument that turns this executable into the agt CLI.
pub(crate) const CLI_FLAG: &str = "--agt-cli";

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    if args.get(1).is_some_and(|arg| arg == CLI_FLAG) {
        let mut cli = vec![args[0].clone()];
        cli.extend(args.into_iter().skip(2));
        return agt_cli::main_from(cli);
    }
    app::run()
}

/// Desktop text follows the system language, like `agt web`'s terminal output.
pub(crate) fn say(zh: &str, en: &str) -> String {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default();
    let chinese = locale
        .get(..2)
        .is_some_and(|p| p.eq_ignore_ascii_case("zh"));
    if chinese { zh } else { en }.to_owned()
}
