//! ttwmctl - Command-line interface to control ttwm
//!
//! This tool allows external programs (including coding agents) to:
//! - Query WM state
//! - Execute actions (focus, split, etc.)
//! - Capture screenshots
//! - Validate state invariants
//!
//! # Examples
//!
//! ```bash
//! # Get full state as JSON
//! ttwmctl state
//!
//! # Get list of windows
//! ttwmctl windows
//!
//! # Focus a specific window
//! ttwmctl focus 12345
//!
//! # Split the focused frame
//! ttwmctl split horizontal
//!
//! # Capture a screenshot
//! ttwmctl screenshot /tmp/test.png
//!
//! # Validate state invariants
//! ttwmctl validate
//! ```

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use serde_json::Value;

/// Get the socket path for this display
fn socket_path() -> PathBuf {
    let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_string());
    let sanitized = display.replace([':', '.'], "_");
    PathBuf::from(format!("/tmp/ttwm{}.sock", sanitized))
}

/// ttwmctl - Control ttwm window manager
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Socket path (default: /tmp/ttwm_$DISPLAY.sock)
    #[arg(long, global = true)]
    socket: Option<PathBuf>,

    /// Output raw JSON without pretty-printing
    #[arg(long, global = true)]
    raw: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Get full WM state as JSON
    State,

    /// Get layout tree as JSON
    Layout,

    /// Get list of all managed windows
    Windows,

    /// Get currently focused window ID
    Focused,

    /// Validate WM state invariants
    Validate,

    /// Get recent event log
    EventLog {
        /// Number of events to retrieve
        #[arg(short, long)]
        count: Option<usize>,
    },

    /// Focus a specific window by ID
    Focus {
        /// Window ID (decimal or hex with 0x prefix)
        window: String,
    },

    /// Focus a specific tab by index (1-based)
    FocusTab {
        /// Tab index (1-based)
        index: usize,
    },

    /// Focus the next or previous frame
    FocusFrame {
        /// Direction: next or prev
        direction: String,
    },

    /// Split the focused frame
    Split {
        /// Direction: horizontal (h) or vertical (v)
        direction: String,
    },

    /// Move the focused window to an adjacent frame
    MoveWindow {
        /// Direction: next or prev
        direction: String,
    },

    /// Resize the focused split
    Resize {
        /// Direction: grow or shrink
        direction: String,
    },

    /// Close the focused window
    Close,

    /// Cycle tabs in the focused frame
    CycleTab {
        /// Direction: next or prev
        #[arg(default_value = "next")]
        direction: String,
    },

    /// Capture a screenshot
    Screenshot {
        /// Path to save the screenshot
        path: PathBuf,
    },

    /// Quit the window manager
    Quit,
}

fn main() {
    let cli = Cli::parse();

    let socket_path = cli.socket.unwrap_or_else(socket_path);

    // Build the command JSON
    let command = match &cli.command {
        Commands::State => serde_json::json!({"command": "get_state"}),
        Commands::Layout => serde_json::json!({"command": "get_layout"}),
        Commands::Windows => serde_json::json!({"command": "get_windows"}),
        Commands::Focused => serde_json::json!({"command": "get_focused"}),
        Commands::Validate => serde_json::json!({"command": "validate_state"}),
        Commands::EventLog { count } => {
            serde_json::json!({"command": "get_event_log", "count": count})
        }
        Commands::Focus { window } => {
            let window_id = parse_window_id(window);
            serde_json::json!({"command": "focus_window", "window": window_id})
        }
        Commands::FocusTab { index } => {
            serde_json::json!({"command": "focus_tab", "index": index})
        }
        Commands::FocusFrame { direction } => {
            let forward = direction.to_lowercase() != "prev";
            serde_json::json!({"command": "focus_frame", "forward": forward})
        }
        Commands::Split { direction } => {
            serde_json::json!({"command": "split", "direction": direction})
        }
        Commands::MoveWindow { direction } => {
            let forward = direction.to_lowercase() != "prev";
            serde_json::json!({"command": "move_window", "forward": forward})
        }
        Commands::Resize { direction } => {
            let delta = if direction.to_lowercase() == "grow" { 0.05 } else { -0.05 };
            serde_json::json!({"command": "resize_split", "delta": delta})
        }
        Commands::Close => serde_json::json!({"command": "close_window"}),
        Commands::CycleTab { direction } => {
            let forward = direction.to_lowercase() != "prev";
            serde_json::json!({"command": "cycle_tab", "forward": forward})
        }
        Commands::Screenshot { path } => {
            serde_json::json!({"command": "screenshot", "path": path.to_string_lossy()})
        }
        Commands::Quit => serde_json::json!({"command": "quit"}),
    };

    // Connect and send command
    match send_command(&socket_path, &command, cli.raw) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

fn parse_window_id(s: &str) -> u32 {
    if let Some(hex) = s.strip_prefix("0x") {
        u32::from_str_radix(hex, 16).unwrap_or_else(|_| {
            eprintln!("Invalid hex window ID: {}", s);
            std::process::exit(1);
        })
    } else {
        s.parse().unwrap_or_else(|_| {
            eprintln!("Invalid window ID: {}", s);
            std::process::exit(1);
        })
    }
}

fn send_command(socket_path: &PathBuf, command: &Value, raw: bool) -> std::io::Result<()> {
    let mut stream = UnixStream::connect(socket_path).map_err(|e| {
        std::io::Error::new(
            e.kind(),
            format!("Failed to connect to ttwm at {:?}: {}. Is ttwm running?", socket_path, e),
        )
    })?;

    // Set timeouts
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    // Send command
    let json = serde_json::to_string(command)?;
    writeln!(stream, "{}", json)?;
    stream.flush()?;

    // Read response
    let mut reader = BufReader::new(&stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;

    // Parse and display response
    let value: Value = serde_json::from_str(&response).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("Invalid JSON response: {}", e))
    })?;

    // Check for error response
    if let Some(status) = value.get("status") {
        if status == "error" {
            let code = value.get("code").and_then(|v| v.as_str()).unwrap_or("unknown");
            let message = value.get("message").and_then(|v| v.as_str()).unwrap_or("Unknown error");
            eprintln!("Error [{}]: {}", code, message);
            std::process::exit(1);
        }
    }

    // Output the response
    if raw {
        println!("{}", response.trim());
    } else {
        let pretty = serde_json::to_string_pretty(&value)?;
        println!("{}", pretty);
    }

    Ok(())
}
