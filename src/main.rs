mod app;
mod cli;
mod config;
mod db;
mod export;
mod import;
mod logging;
mod templates;
mod value;

use anyhow::Result;
use clap::Parser;
use cli::Cli;

fn main() -> Result<()> {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Console::SetConsoleOutputCP;
        let _ = SetConsoleOutputCP(65001); // UTF-8
    }

    let cli = Cli::parse();
    app::run(cli)
}
