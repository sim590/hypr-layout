
use std::io::{BufRead,BufReader};
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::process::Command;
use std::str::FromStr;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::ast::Direction;

pub const TABBED_SPLIT_ERROR: &str = "Tabbed splits require the hy3 plugin to be installed and the default layout set to hy3.";

#[derive(Clone, Copy)]
pub enum LayoutEngine { Hy3, Dwindle }

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

pub struct HyprlandContext {
}

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

impl HyprlandContext {

    // Public methods

    pub fn dispatch(args: &[&str]) -> Result<()> {
        let mut full_args = vec!["dispatch"];
        full_args.extend_from_slice(args);
        hyprctl(&full_args)?;
        Ok(())
    }

    pub fn direction_to_hy3(direction: &Direction) -> &'static str {
        match direction {
            Direction::Horizontal => "h",
            Direction::Vertical   => "v",
            Direction::Tabbed     => "tab",
        }
    }

    pub fn direction_to_dwindle_preselect(direction: &Direction) -> Result<&'static str> {
        let s = match direction {
            Direction::Horizontal => "r",
            Direction::Vertical   => "d",
            Direction::Tabbed     => anyhow::bail!(TABBED_SPLIT_ERROR),
        };
        Ok(s)
    }

    pub fn focus_window(addr: &str) -> Result<()> {
        HyprlandContext::dispatch(&["focuswindow", &format!("address:{addr}")])
    }

    pub fn is_floating(addr: &str) -> Result<bool> {
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

    /// Return the current pixel size of a window looked up by its address.
    pub fn get_window_size(addr: &str) -> Result<(u32, u32)> {
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
    pub fn resize_window_exact(addr: &str, w: u32, h: u32) -> Result<()> {
        HyprlandContext::dispatch(&["resizewindowpixel", &format!("exact {w} {h},address:{addr}")])
    }

    /// Block until a new window is opened, listening on the Hyprland event socket.
    /// Returns the window address in "0xADDRESS" format.
    pub fn wait_for_window(mut child_process: Option<std::process::Child>, timeout: Duration, command: &str) -> Result<String> {
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

                    if HyprlandContext::is_floating(&addr_str)? {
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

    // Private methods

    pub fn detect_layout_engine() -> Result<LayoutEngine> {
        let general_layout_json               = hyprctl(&["getoption", "general:layout", "-j"])?;
        let general_layout: serde_json::Value = serde_json::from_slice(&general_layout_json)?;
        LayoutEngine::from_str(general_layout["str"].as_str().unwrap_or(""))
    }
}

