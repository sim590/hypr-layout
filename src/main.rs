
mod ast;
mod layout;
mod builder;

use std::env;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::process::Command;
use std::time::Duration;

use clap::Parser;
use clap::error::ErrorKind;

use crate::ast::Layout;
use crate::builder::build;

// Récupérer la variable d'environnement $HOME
static HOME_DIR: LazyLock<String> = LazyLock::new(|| {
    env::var("HOME").expect("Failed to get HOME environment variable")
});

fn notify_error(msg: &str) {
    let _ = Command::new("hyprctl").args(["notify", "3", "7000", "rgb(ff3333)", &format!("fontsize:14 {msg}")])
                                   .status();
}

#[derive(Parser)]
struct Args {
    /// The terminal emulator to use for leaf nodes. Default is "alacritty".
    #[arg(long, default_value = "alacritty")]
    terminal: String,

    /// The working directory for the terminal emulator. If not specified, it
    /// will use the current working directory.
    #[arg(long)]
    cwd: Option<PathBuf>,

    /// The layout of the containers. This should be a string representation of
    /// the container structure, for example:
    ///   t(h(v(30%:{ranger}, {tig -w}), 30%:{}), h({vim}, {opencode}))
    layout: Layout,

    /// Optional timeout in milliseconds for the layout application. If not specified,
    /// it will default to 5000 ms (5 seconds).
    #[arg(long, default_value = "5000")]
    timeout: u64,
}

fn main() {
    let args = Args::try_parse().unwrap_or_else(|e| {
        e.print().ok();

        if !matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
            notify_error(&e.to_string());
        }
        std::process::exit(e.exit_code());
    });

    let cwd = args.cwd.unwrap_or_else(|| PathBuf::from(HOME_DIR.as_str()));

    if let Err(e) = build(&args.layout, &args.terminal, &cwd, Duration::from_millis(args.timeout)) {
        let msg = format!("hypr-layout: {e:#}");
        eprintln!("{msg}");
        notify_error(&msg);
        std::process::exit(1);
    }
}

