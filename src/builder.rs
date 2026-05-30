
use std::path::Path;
use std::time::Duration;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use which::which;

use crate::ast::{Direction, Layout};
use crate::hyprland::{HyprlandContext, LayoutEngine, TABBED_SPLIT_ERROR};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn build(layout: &Layout, terminal: &str, cwd: &Path, timeout: Duration) -> Result<()> {
    let ctx = HyprlandContext::detect()?;
    validate(layout, &ctx)?;
    let first_addr = launch_first_leaf(layout, terminal, cwd, timeout)?;
    build_recursive(layout, &first_addr, terminal, cwd, timeout, &ctx)
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------
fn validate(layout: &Layout, ctx: &HyprlandContext) -> Result<()> {
    match (layout, ctx.layout_engine) {
        (Layout::Split { direction: Direction::Tabbed, .. }, LayoutEngine::Dwindle) => anyhow::bail!(TABBED_SPLIT_ERROR),
        (Layout::Split { children, .. }, _)                                         => children.iter().try_for_each(|(_, child)| validate(child, ctx)),
        (Layout::Leaf { command: Some(c), .. }, _)                                  => {
            let p = parse_program_from_cmd(c)?;
            which(p).with_context(|| p.to_string())?;
            Ok(())
        },
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

fn parse_program_from_cmd(cmd: &str) -> Result<&str> {
    let s = cmd.trim_start();
    if let Some(q) = s.chars().next().filter(|c| matches!(c, '"' | '\'')) {
        s[1..].split_once(q)
              .map(|(prog, _)| prog)
              .with_context(|| format!("The program in the command couldn't be parsed correctly from '{cmd}'"))
    } else {
        let end = s.find(' ').unwrap_or(s.len());
        Ok(&s[..end])
    }
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
            HyprlandContext::wait_for_window(Some(child_process), timeout, label)
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
    ctx:             &HyprlandContext,
) -> Result<()> {
    match node {
        Layout::Split { direction, children } => {
            if children.is_empty() {
                anyhow::bail!("Split node has no children");
            }

            match (direction, ctx.layout_engine) {
                (Direction::Tabbed, LayoutEngine::Hy3) => {
                    ctx.hy3_change_group("tab")?;
                    ctx.focus_window(first_leaf_addr)?;

                    // Pre-launch the first leaf of every remaining tab.
                    let mut child_addrs = vec![first_leaf_addr.to_string()];
                    for (_, child) in children[1..].iter() {
                        let addr = launch_first_leaf(child, terminal, cwd, timeout)?;
                        child_addrs.push(addr);
                    }

                    // Build the internal structure of each tab.
                    for (i, (_, child)) in children.iter().enumerate() {
                        ctx.focus_window(&child_addrs[i])?;
                        build_recursive(child, &child_addrs[i], terminal, cwd, timeout, ctx)?;
                    }

                    Ok(())
                }
                (Direction::Tabbed, _) => anyhow::bail!(TABBED_SPLIT_ERROR),
                _ => {
                    if let LayoutEngine::Hy3 = ctx.layout_engine {
                        ctx.hy3_make_group(direction)?;
                    }
                    ctx.focus_window(first_leaf_addr)?;

                    // Pre-launch the first leaf of every remaining child.
                    // In dwindle mode, each sibling is placed with preselect
                    // relative to the previous one. In hy3 mode, this ensures
                    // the group always has multiple members before recursing.
                    let mut child_addrs = vec![first_leaf_addr.to_string()];
                    for (last_child_i, (_, child)) in children[1..].iter().enumerate() {
                        if let LayoutEngine::Dwindle = ctx.layout_engine {
                            let preselect_d = HyprlandContext::direction_to_dwindle_preselect(direction)?;

                            ctx.focus_window(&child_addrs[last_child_i])?;
                            ctx.layoutmsg(&format!("preselect {preselect_d}"))?;
                        }
                        let addr = launch_first_leaf(child, terminal, cwd, timeout)?;
                        child_addrs.push(addr);
                    }

                    // Build the internal structure of each child.
                    for (i, (_, child)) in children.iter().enumerate() {
                        ctx.focus_window(&child_addrs[i])?;
                        build_recursive(child, &child_addrs[i], terminal, cwd, timeout, ctx)?;
                    }

                    // Apply size ratios if at least one child specifies one.
                    let ratios: Vec<u8> = children.iter().map(|(r, _)| *r).collect();
                    if ratios.iter().any(|&r| r > 0) {
                        apply_split_ratios(direction, &ratios, &child_addrs, ctx)?;
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
    ctx:       &HyprlandContext,
) -> Result<()> {
    debug_assert_eq!(ratios.len(), addrs.len());
    let n = ratios.len();

    // No explicit ratio — the layout engine already distributes space evenly.
    if ratios.iter().all(|&r| r == 0) {
        return Ok(());
    }

    let effective = normalize_ratios(ratios)?;

    // Snapshot all window sizes before any resize operation.
    let sizes: Vec<(u32, u32)> = addrs.iter()
        .map(|addr| HyprlandContext::get_window_size(addr))
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
        ctx.resize_window_exact(&addrs[i], target_w, target_h)?;
    }

    Ok(())
}

/// Normalize a slice of ratios: explicit (non-zero) values are kept as-is,
/// implicit (zero) values receive an equal share of the remaining percentage.
/// Returns an error if the explicit ratios exceed 100%.
fn normalize_ratios(ratios: &[u8]) -> Result<Vec<u32>> {
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
    Ok(ratios.iter()
        .map(|&r| if r > 0 { r as u32 } else { implicit_ratio })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- shell_escape -------------------------------------------------------

    #[test]
    fn shell_escape_simple_string() {
        assert_eq!(shell_escape("hello"), "'hello'");
    }

    #[test]
    fn shell_escape_empty_string() {
        assert_eq!(shell_escape(""), "''");
    }

    #[test]
    fn shell_escape_with_single_quote() {
        assert_eq!(shell_escape("l'été"), "'l'\\''été'");
    }

    #[test]
    fn shell_escape_with_spaces() {
        assert_eq!(shell_escape("/home/user/my dir"), "'/home/user/my dir'");
    }

    // --- parse_program_from_cmd ---------------------------------------------

    #[test]
    fn parse_program_simple() {
        assert_eq!(parse_program_from_cmd("vim").unwrap(), "vim");
    }

    #[test]
    fn parse_program_with_args() {
        assert_eq!(parse_program_from_cmd("vim -c 'set nu'").unwrap(), "vim");
    }

    #[test]
    fn parse_program_double_quoted_with_spaces() {
        assert_eq!(parse_program_from_cmd("\"/home/the_user/His Custom Directory/vim\" -c args").unwrap(), "/home/the_user/His Custom Directory/vim");
    }

    #[test]
    fn parse_program_double_quoted() {
        assert_eq!(parse_program_from_cmd("\"vim\" -c args").unwrap(), "vim");
    }

    #[test]
    fn parse_program_single_quoted() {
        assert_eq!(parse_program_from_cmd("'vim' -c args").unwrap(), "vim");
    }

    #[test]
    fn parse_program_leading_whitespace() {
        assert_eq!(parse_program_from_cmd("  vim").unwrap(), "vim");
    }

    #[test]
    fn parse_program_unclosed_quote_rejected() {
        assert!(parse_program_from_cmd("\"unclosed").is_err());
    }

    // --- normalize_ratios ---------------------------------------------------

    #[test]
    fn normalize_ratios_all_explicit() {
        assert_eq!(normalize_ratios(&[30, 70]).unwrap(), vec![30, 70]);
    }

    #[test]
    fn normalize_ratios_some_implicit() {
        // 30 explicit + 2 implicit → each implicit gets (100-30)/2 = 35
        assert_eq!(normalize_ratios(&[30, 0, 0]).unwrap(), vec![30, 35, 35]);
    }

    #[test]
    fn normalize_ratios_all_implicit() {
        // 3 implicit → each gets 100/3 = 33 (integer division)
        assert_eq!(normalize_ratios(&[0, 0, 0]).unwrap(), vec![33, 33, 33]);
    }

    #[test]
    fn normalize_ratios_single_explicit() {
        assert_eq!(normalize_ratios(&[100]).unwrap(), vec![100]);
    }

    #[test]
    fn normalize_ratios_exceeds_100_rejected() {
        assert!(normalize_ratios(&[60, 50]).is_err());
    }

    #[test]
    fn normalize_ratios_exactly_100() {
        assert_eq!(normalize_ratios(&[25, 25, 50]).unwrap(), vec![25, 25, 50]);
    }
}
