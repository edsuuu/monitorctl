mod cli;
mod ddc;
mod displays;
mod features;
mod gui;
mod log;
mod logo;
mod mcp;
mod platform;
mod unsaved;

use crate::log::{Logged, app_folder, log};

fn main() {
    std::panic::set_hook(Box::new(|panic| {
        log(format!("PANIC {panic}\n{}", std::backtrace::Backtrace::force_capture()));
        eprintln!("monitorctl: {panic} (detalhes em {})", app_folder().join("monitorctl.log").display());
    }));
    platform::crash_handler();
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    log(format!("monitorctl {} ({}) args={arguments:?}", env!("CARGO_PKG_VERSION"), std::env::consts::OS));
    // --min: started by "iniciar com o sistema"
    let start_minimized = arguments.iter().any(|argument| argument == "--min");
    if !arguments.is_empty() && !start_minimized {
        match cli::run(&arguments).logged(&format!("cli {arguments:?}")) {
            Ok(output) if output.is_empty() => {}
            Ok(output) => println!("{}", output.trim_end()),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    platform::hide_console();
    gui::run(start_minimized);
}
