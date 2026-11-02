#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    // `SectorForge.exe sf-cli <args>` runs the embedded engine instead of opening a window.
    // The GUI starts itself this way for every job, so Windows needs only this one exe.
    #[cfg(windows)]
    if std::env::args_os().nth(1).is_some_and(|a| a == sectorforge_gui_lib::EMBEDDED_ARG) {
        return sf_cli::run(std::iter::once(std::ffi::OsString::from("sf-cli")).chain(std::env::args_os().skip(2)));
    }
    sectorforge_gui_lib::run();
    std::process::ExitCode::SUCCESS
}
