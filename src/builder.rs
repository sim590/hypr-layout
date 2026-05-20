
use std::path::Path;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::process::Command;

use anyhow::{Context, Result};

use crate::ast::{Direction, Layout};

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

fn focus_window(addr: &str) -> Result<()> {
    dispatch(&["focuswindow", &format!("address:{addr}")])
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn build(layout: &Layout, terminal: &str, cwd: &Path) -> Result<()> {
    let first_addr = launch_first_leaf(layout, terminal, cwd)?;
    build_recursive(layout, &first_addr, terminal, cwd)
}

// ---------------------------------------------------------------------------
// Window launching
// ---------------------------------------------------------------------------

/// Launch the command associated with the first leaf of a node, wait for its
/// window to appear, and return the Hyprland window address.
fn launch_first_leaf(node: &Layout, terminal: &str, cwd: &Path) -> Result<String> {
    match node {
        Layout::Split { children, .. } => {
            if let Some((_, first_child)) = children.first() {
                launch_first_leaf(first_child, terminal, cwd)
            } else {
                anyhow::bail!("Split node has no children");
            }
        }
        Layout::Leaf { command, terminal: is_terminal } => {
            let cmd = match (is_terminal, command.as_deref()) {
                (false, Some(c)) => c.to_string(),
                (true,  Some(c)) => format!("{terminal} --working-directory {} -e {c}", cwd.display()),
                (true,  None)    => format!("{terminal} --working-directory {}", cwd.display()),
                (_,     None)    => anyhow::bail!("Leaf node has no command to execute!"),
            };
            dispatch(&["exec", &cmd])?;
            wait_for_window()
        }
    }
}

/// Block until a new window is opened, listening on the Hyprland event socket.
/// Returns the window address in "0xADDRESS" format.
fn wait_for_window() -> Result<String> {
    let sig            = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")?;
    let xdg_runtime    = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/user/1000".to_string());
    let connector_path = format!("{xdg_runtime}/hypr/{sig}/.socket2.sock");
    let mut stream     = UnixStream::connect(&connector_path).context("Failed to connect to Hyprland socket")?;
    let mut reader     = BufReader::new(&mut stream);

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            anyhow::bail!("Hyprland socket closed unexpectedly");
        } else if line.starts_with("openwindow>>") {
            // Event format: openwindow>>ADDR,WORKSPACE,CLASS,TITLE
            // ADDR does not carry the "0x" prefix in the event payload.
            let addr = line
                .trim_start_matches("openwindow>>")
                .split(',')
                .next()
                .context("Failed to parse openwindow event")?
                .trim()
                .to_string();
            return Ok(format!("0x{addr}"));
        }
    }
}

// ---------------------------------------------------------------------------
// Layout building
// ---------------------------------------------------------------------------

/// Recursively build the hy3 layout.
///
/// Precondition: the first leaf of `node` has already been launched and is
/// focused; its Hyprland address is `first_leaf_addr`.
///
/// Strategy shared by both the h/v and tabbed branches:
///   1. Pre-launch the first leaf of ALL remaining children before building
///      the internal structure of the first child. This ensures the parent
///      group always has multiple members, preventing hy3 from bubbling an
///      exec past the group boundary (e.g. into a parent tabbed group) when
///      the first child is temporarily the only member.
///   2. Build each child's internal structure by navigating with
///      `focus_window` — no `changefocus raise` needed.
fn build_recursive(
    node:            &Layout,
    first_leaf_addr: &str,
    terminal:        &str,
    cwd:             &Path,
) -> Result<()> {
    match node {
        Layout::Split { direction, children } => {
            if children.is_empty() {
                anyhow::bail!("Split node has no children");
            }

            match direction {
                Direction::Tabbed => {
                    dispatch(&["hy3:changegroup", "tab"])?;
                    focus_window(first_leaf_addr)?;

                    // Pre-launch the first leaf of every remaining tab.
                    let mut child_addrs = vec![first_leaf_addr.to_string()];
                    for (_, child) in children[1..].iter() {
                        let addr = launch_first_leaf(child, terminal, cwd)?;
                        child_addrs.push(addr);
                    }

                    // Build the internal structure of each tab.
                    for (i, (_, child)) in children.iter().enumerate() {
                        focus_window(&child_addrs[i])?;
                        build_recursive(child, &child_addrs[i], terminal, cwd)?;
                    }

                    Ok(())
                }
                _ => {
                    dispatch(&["hy3:makegroup", direction_to_hy3(direction)])?;
                    focus_window(first_leaf_addr)?;

                    // Pre-launch the first leaf of every remaining child so that
                    // the h/v group has all its members from the start. Without
                    // this, an exec inside the first child (when it is the sole
                    // member) can escape to a parent tabbed group instead of
                    // opening as a sibling.
                    let mut child_addrs = vec![first_leaf_addr.to_string()];
                    for (_, child) in children[1..].iter() {
                        let addr = launch_first_leaf(child, terminal, cwd)?;
                        child_addrs.push(addr);
                    }

                    // Build the internal structure of each child.
                    for (i, (_, child)) in children.iter().enumerate() {
                        focus_window(&child_addrs[i])?;
                        build_recursive(child, &child_addrs[i], terminal, cwd)?;
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

    // No explicit ratio — hy3 already distributes space evenly.
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
