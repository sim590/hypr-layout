
use std::str::FromStr;
use std::path::Path;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::time::Duration;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

use anyhow::{Context, Result};

use crate::ast::{Direction, Layout};

const TABBED_SPLIT_ERROR: &str = "Tabbed splits require the hy3 plugin to be installed and the default layout set to hy3.";

// ---------------------------------------------------------------------------
// hyprctl wrappers
// ---------------------------------------------------------------------------

fn hyprctl(args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("hyprctl")
        .args(args)
        .output()
        .context("Failed to launch hyprctl")?;
    if !output.status.success() {
        anyhow::bail!("hyprctl failed: {}", output.status);
    }
    Ok(output.stdout)
}

fn dispatch(args: &[&str]) -> Result<()> {
    let mut full_args = vec!["dispatch"];
    full_args.extend_from_slice(args);
    hyprctl(&full_args)?;
    Ok(())
}

fn direction_to_hy3(direction: &Direction) -> &'static str {
    match direction {
        Direction::Horizontal => "h",
        Direction::Vertical   => "v",
        Direction::Tabbed     => "tab",
    }
}

fn direction_to_dwindle_preselect(direction: &Direction) -> Result<&'static str> {
    let s = match direction {
        Direction::Horizontal => "r",
        Direction::Vertical   => "d",
        Direction::Tabbed     => anyhow::bail!(TABBED_SPLIT_ERROR),
    };
    Ok(s)
}

fn focus_window(addr: &str) -> Result<()> {
    dispatch(&["focuswindow", &format!("address:{addr}")])
}

fn is_floating(addr: &str) -> Result<bool> {
    for _ in 0..5 {
        let clients_json                    = hyprctl(&["clients", "-j"])?;
        let clients: Vec<serde_json::Value> = serde_json::from_slice(&clients_json)?;
        if let Some(client) = clients.iter().find(|v| v["address"].as_str() == Some(addr) ) {
            return Ok(client["floating"].as_bool() == Some(true));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(false)
}

enum LayoutEngine { Hy3, Dwindle }
impl FromStr for LayoutEngine {
    type Err = anyhow::Error;
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input {
            "hy3"     => Ok(LayoutEngine::Hy3),
            "dwindle" => Ok(LayoutEngine::Dwindle),
            _         => anyhow::bail!("Unsupported Layout: {input}. hypr-layout only supports hy3 and dwindle.")
        }
    }
}

fn detect_layout_engine() -> Result<LayoutEngine> {
    let general_layout_json               = hyprctl(&["getoption", "general:layout", "-j"])?;
    let general_layout: serde_json::Value = serde_json::from_slice(&general_layout_json)?;
    LayoutEngine::from_str(general_layout["str"].as_str().unwrap_or(""))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn build(layout: &Layout, terminal: &str, cwd: &Path, timeout: Duration) -> Result<()> {
    let layout_engine = detect_layout_engine()?;
    validate_layout(layout, &layout_engine)?;
    let first_addr = launch_first_leaf(layout, terminal, cwd, timeout)?;
    build_recursive(layout, &first_addr, terminal, cwd, timeout, &layout_engine)
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------
fn validate_layout(layout: &Layout, layout_engine: &LayoutEngine) -> Result<()> {
    match (layout, layout_engine) {
        (Layout::Split { direction: Direction::Tabbed, .. }, LayoutEngine::Dwindle) => anyhow::bail!(TABBED_SPLIT_ERROR),
        (Layout::Split { children, .. }, _)                                         => children.iter().try_for_each(|(_, child)| validate_layout(child, layout_engine)),
        _                                                                           => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Terminal conventions
// ---------------------------------------------------------------------------

/// Wraps a string in single quotes, escaping any inner single quotes.
/// e.g. `l'été` → `'l'\''été'`.
/// Use only for strings embedded inside a `sh -c` argument.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn build_command(command: &Option<String>, cwd: &Path, terminal: &str) -> Command {
    let mut cmd = Command::new(terminal);

    // If a command is provided, use it directly — it is already a shell
    // expression and must not be quoted. Otherwise fall back to $SHELL so
    // that sh expands the variable itself.
    let exec_arg = match command.as_deref() {
        Some(c) => format!("exec {c}"),
        None    => "exec $SHELL".to_string(),
    };

    match terminal {
        "alacritty"
      | "ghostty"
      | "xfce4-terminal"
      | "tilix"
      | "sakura"
      | "terminator"
      | "rio" => {
            cmd.arg("--working-directory").arg(cwd)
               .arg("-e").arg("sh").arg("-c").arg(&exec_arg);
        }
        "kitty" => {
            cmd.arg("--directory").arg(cwd)
               .arg("--").arg("sh").arg("-c").arg(&exec_arg);
        }
        "foot" => {
            // foot passes the command as positional arguments (no -e flag)
            cmd.arg("--working-directory").arg(cwd)
               .arg("sh").arg("-c").arg(&exec_arg);
        }
        "gnome-terminal" => {
            cmd.arg("--working-directory").arg(cwd)
               .arg("--").arg("sh").arg("-c").arg(&exec_arg);
        }
        "konsole" => {
            cmd.arg("--workdir").arg(cwd)
               .arg("-e").arg("sh").arg("-c").arg(&exec_arg);
        }
        "wezterm" => {
            // wezterm requires the "start" subcommand
            cmd.arg("start").arg("--cwd").arg(cwd)
               .arg("--").arg("sh").arg("-c").arg(&exec_arg);
        }
        _ => {
            // Fallback: no guaranteed --working-directory flag; embed the
            // cd into the sh -c string instead.
            let cwd_esc = shell_escape(&cwd.display().to_string());
            cmd.arg("-e").arg("sh").arg("-c")
               .arg(format!("cd {cwd_esc} && {exec_arg}"));
        }
    }
    cmd
}

// ---------------------------------------------------------------------------
// Window launching
// ---------------------------------------------------------------------------

/// Launch the command associated with the first leaf of a node, wait for its
/// window to appear, and return the Hyprland window address.
fn launch_first_leaf(node: &Layout, terminal: &str, cwd: &Path, timeout: Duration) -> Result<String> {
    match node {
        Layout::Split { children, .. } => {
            if let Some((_, first_child)) = children.first() {
                launch_first_leaf(first_child, terminal, cwd, timeout)
            } else {
                anyhow::bail!("Split node has no children");
            }
        }
        Layout::Leaf { command, terminal: is_terminal } => {
            let (child_process, label) = match (is_terminal, command.as_deref()) {
                (false, Some(c)) => (Command::new("setsid").args(["sh", "-c", c]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?, c),
                (true,  Some(c)) => (build_command(command, cwd, terminal).spawn()?, c),
                (true,  None)    => (build_command(command, cwd, terminal).spawn()?, terminal),
                (_,     None)    => anyhow::bail!("Leaf node has no command to execute!"),
            };
            wait_for_window(Some(child_process), timeout, label)
        }
    }
}

/// Block until a new window is opened, listening on the Hyprland event socket.
/// Returns the window address in "0xADDRESS" format.
fn wait_for_window(mut child_process: Option<std::process::Child>, timeout: Duration, command: &str) -> Result<String> {
    static XDG_RUNTIME: LazyLock<String> = LazyLock::new(|| {
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| {
            let uid = std::process::Command::new("id").arg("-u").output()
                                                                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                                                                .expect("impossible d'obtenir l'UID");
            format!("/run/user/{uid}")
        })
    });

    let sig            = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")?;
    let connector_path = format!("{}/hypr/{sig}/.socket2.sock", XDG_RUNTIME.as_str());
    let mut stream     = UnixStream::connect(&connector_path).context("Failed to connect to Hyprland socket")?;

    stream.set_read_timeout(Some(Duration::from_millis(100)))?;

    let mut reader = BufReader::new(&mut stream);
    let start      = std::time::Instant::now();


    loop {
        let mut line = String::new();

        match reader.read_line(&mut line) {
            Ok(0) => anyhow::bail!("Hyprland socket closed unexpectedly"),
            Ok(_) if line.starts_with("openwindow>>") => {
                // Event format: openwindow>>ADDR,WORKSPACE,CLASS,TITLE
                // ADDR does not carry the "0x" prefix in the event payload.
                let addr = line.trim_start_matches("openwindow>>")
                               .split(',')
                               .next()
                               .context("Failed to parse openwindow event")?
                               .trim()
                               .to_string();
                let addr_str = format!("0x{addr}");

                if is_floating(&addr_str)? {
                    continue;
                }

                return Ok(addr_str);
            },
            Ok(_) => {},
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                   || e.kind() == std::io::ErrorKind::TimedOut => {},
            Err(e) => return Err(e.into()),
        }

        if let Some(c) = child_process.as_mut() && let Some(status) = c.try_wait()? {
            if let Some(sig) = status.signal() {
                anyhow::bail!(
                    "'{command}' was killed by signal {sig} before creating a window.\n\
                     If it is a TUI app, perhaps make sure to use '{{{command}}}' to run it inside a terminal."
                );
            }
            // Normal exit, stop monitoring, keep waiting for window
            child_process = None;
        }

        if start.elapsed() > timeout {
            anyhow::bail!(
                "Timed out after {}s waiting for '{command}' to open a window.\n\
                 Possible causes: the command crashed, it is a TUI app (try '{{{command}}}'),\n\
                 or it is a singleton app that is already running.",
                timeout.as_secs()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Layout building
// ---------------------------------------------------------------------------

/// Recursively build the layout (hy3 or dwindle).
///
/// Precondition: the first leaf of `node` has already been launched and is
/// focused; its Hyprland address is `first_leaf_addr`.
///
/// Strategy shared by both the h/v and tabbed branches:
///   1. Pre-launch the first leaf of ALL remaining children before building
///      the internal structure of the first child.
///      - hy3: prevents the parent group from having a single member, which
///        would cause hy3 to bubble an exec past the group boundary.
///      - dwindle: each sibling is placed with `preselect` relative to the
///        previous one, anchoring the split boundary before recursing.
///   2. Build each child's internal structure by navigating with
///      `focus_window` — no `changefocus raise` needed.
fn build_recursive(
    node:            &Layout,
    first_leaf_addr: &str,
    terminal:        &str,
    cwd:             &Path,
    timeout:         Duration,
    layout_engine:   &LayoutEngine,
) -> Result<()> {
    match node {
        Layout::Split { direction, children } => {
            if children.is_empty() {
                anyhow::bail!("Split node has no children");
            }

            match (direction, layout_engine) {
                (Direction::Tabbed, LayoutEngine::Hy3) => {
                    dispatch(&["hy3:changegroup", "tab"])?;
                    focus_window(first_leaf_addr)?;

                    // Pre-launch the first leaf of every remaining tab.
                    let mut child_addrs = vec![first_leaf_addr.to_string()];
                    for (_, child) in children[1..].iter() {
                        let addr = launch_first_leaf(child, terminal, cwd, timeout)?;
                        child_addrs.push(addr);
                    }

                    // Build the internal structure of each tab.
                    for (i, (_, child)) in children.iter().enumerate() {
                        focus_window(&child_addrs[i])?;
                        build_recursive(child, &child_addrs[i], terminal, cwd, timeout, layout_engine)?;
                    }

                    Ok(())
                }
                (Direction::Tabbed, _) => anyhow::bail!(TABBED_SPLIT_ERROR),
                _ => {
                    if let LayoutEngine::Hy3 = layout_engine {
                        dispatch(&["hy3:makegroup", direction_to_hy3(direction)])?;
                    }
                    focus_window(first_leaf_addr)?;

                    // Pre-launch the first leaf of every remaining child.
                    // In dwindle mode, each sibling is placed with preselect
                    // relative to the previous one. In hy3 mode, this ensures
                    // the group always has multiple members before recursing.
                    let mut child_addrs = vec![first_leaf_addr.to_string()];
                    for (last_child_i, (_, child)) in children[1..].iter().enumerate() {
                        if let LayoutEngine::Dwindle = layout_engine {
                            focus_window(&child_addrs[last_child_i])?;
                            dispatch(&["layoutmsg", "preselect", direction_to_dwindle_preselect(direction)?])?;
                        }
                        let addr = launch_first_leaf(child, terminal, cwd, timeout)?;
                        child_addrs.push(addr);
                    }

                    // Build the internal structure of each child.
                    for (i, (_, child)) in children.iter().enumerate() {
                        focus_window(&child_addrs[i])?;
                        build_recursive(child, &child_addrs[i], terminal, cwd, timeout, layout_engine)?;
                    }

                    // Apply size ratios if at least one child specifies one.
                    let ratios: Vec<u8> = children.iter().map(|(r, _)| *r).collect();
                    if ratios.iter().any(|&r| r > 0) {
                        apply_split_ratios(direction, &ratios, &child_addrs)?;
                    }

                    Ok(())
                }
            }
        }
        Layout::Leaf { .. } => Ok(()),
    }
}

/// Apply size ratios to the children of an h/v split node.
///
/// Children are processed left-to-right, skipping the last one. Each resize
/// moves the border toward the right/bottom neighbour, so already-resized
/// children are not disturbed.
fn apply_split_ratios(
    direction: &Direction,
    ratios:    &[u8],
    addrs:     &[String],
) -> Result<()> {
    debug_assert_eq!(ratios.len(), addrs.len());
    let n = ratios.len();

    // No explicit ratio — the layout engine already distributes space evenly.
    if ratios.iter().all(|&r| r == 0) {
        return Ok(());
    }

    // Normalize: fill implicit (zero) ratios with an equal share of what remains.
    let sum_explicit: u32 = ratios.iter().filter(|&&r| r > 0).map(|&r| r as u32).sum();
    if sum_explicit > 100 {
        anyhow::bail!("Ratios exceed 100% (sum = {}%)", sum_explicit);
    }
    let count_implicit = ratios.iter().filter(|&&r| r == 0).count();
    let implicit_ratio = if count_implicit > 0 {
        (100 - sum_explicit) / count_implicit as u32
    } else {
        0
    };
    let effective: Vec<u32> = ratios.iter()
        .map(|&r| if r > 0 { r as u32 } else { implicit_ratio })
        .collect();

    // Snapshot all window sizes before any resize operation.
    let sizes: Vec<(u32, u32)> = addrs.iter()
        .map(|addr| get_window_size(addr))
        .collect::<Result<Vec<_>>>()?;

    // Compute the total extent of the group along the split axis.
    let (total_w, total_h) = match direction {
        Direction::Horizontal => {
            let total_w: u32 = sizes.iter().map(|(w, _)| w).sum();
            let h = sizes[0].1;
            (total_w, h)
        }
        Direction::Vertical => {
            let w = sizes[0].0;
            let total_h: u32 = sizes.iter().map(|(_, h)| h).sum();
            (w, total_h)
        }
        Direction::Tabbed => return Ok(()),
    };

    // Resize every child except the last; it fills whatever space remains.
    for i in 0..n - 1 {
        let target_w = match direction {
            Direction::Horizontal => total_w * effective[i] / 100,
            _                     => total_w,
        };
        let target_h = match direction {
            Direction::Vertical => total_h * effective[i] / 100,
            _                   => total_h,
        };
        resize_window_exact(&addrs[i], target_w, target_h)?;
    }

    Ok(())
}

/// Return the current pixel size of a window looked up by its address.
fn get_window_size(addr: &str) -> Result<(u32, u32)> {
    let stdout  = hyprctl(&["clients", "-j"])?;
    let clients: Vec<serde_json::Value> = serde_json::from_slice(&stdout)?;
    for client in clients.iter() {
        if client["address"].as_str() == Some(addr) {
            let w = client["size"][0].as_u64().context("Missing width")? as u32;
            let h = client["size"][1].as_u64().context("Missing height")? as u32;
            return Ok((w, h));
        }
    }
    anyhow::bail!("Window not found in clients list: {}", addr)
}

/// Resize a window to an exact pixel size.
fn resize_window_exact(addr: &str, w: u32, h: u32) -> Result<()> {
    dispatch(&["resizewindowpixel", &format!("exact {w} {h},address:{addr}")])
}
