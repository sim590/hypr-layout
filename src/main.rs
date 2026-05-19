
mod ast;
mod layout;
mod builder;

use std::env;
use std::path::PathBuf;
use std::sync::LazyLock;

use clap::Parser;

use crate::ast::Layout;
use crate::builder::build;

// Récupérer la variable d'environnement $HOME
static HOME_DIR: LazyLock<String> = LazyLock::new(|| {
    env::var("HOME").expect("Failed to get HOME environment variable")
});


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
    ///   t(h(v(30%:{ranger}, {"tig -w"}), 30%:{}), h({vim}, {opencode}))
    layout: Layout
}

fn main() {
    let args = Args::parse();
    let cwd  = args.cwd.unwrap_or_else(|| PathBuf::from(HOME_DIR.as_str()));

    build(&args.layout, &args.terminal, &cwd).expect("Failed to build layout!");
}

