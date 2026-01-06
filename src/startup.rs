//! Startup layout and app spawning management.
//!
//! This module handles applying startup configurations to workspaces
//! and spawning initial applications.

use std::process::Command;

use crate::config::StartupConfig;
use crate::layout::NodeId;
use crate::workspaces::{Workspace, NUM_WORKSPACES};

/// Information about a pending app spawn
#[derive(Debug, Clone)]
pub struct PendingSpawn {
    /// The command to execute
    pub command: String,
    /// Target workspace index (0-based)
    pub workspace_idx: usize,
    /// Target frame NodeId within that workspace
    pub frame_id: NodeId,
    /// Target frame name (for logging)
    pub frame_name: Option<String>,
}

/// Manages startup app spawning and window placement
pub struct StartupManager {
    /// Apps waiting to be spawned
    pending_spawns: Vec<PendingSpawn>,
    /// Whether startup phase is complete
    startup_complete: bool,
}

impl StartupManager {
    pub fn new() -> Self {
        Self {
            pending_spawns: Vec::new(),
            startup_complete: false,
        }
    }

    /// Apply startup configuration to workspaces
    /// Returns list of apps to spawn with their target frame info
    pub fn apply_config(
        &mut self,
        config: &StartupConfig,
        workspaces: &mut [Workspace; NUM_WORKSPACES],
    ) -> Vec<PendingSpawn> {
        let mut all_spawns = Vec::new();

        for (workspace_num_str, ws_config) in &config.workspace {
            // Parse workspace number from string key
            let workspace_num: usize = match workspace_num_str.parse() {
                Ok(n) => n,
                Err(_) => {
                    log::warn!(
                        "Invalid workspace key '{}' in startup config (must be 1-{})",
                        workspace_num_str,
                        NUM_WORKSPACES
                    );
                    continue;
                }
            };

            // Workspace numbers in config are 1-indexed
            let ws_idx = workspace_num.saturating_sub(1);
            if workspace_num < 1 || ws_idx >= NUM_WORKSPACES {
                log::warn!(
                    "Invalid workspace number {} in startup config (must be 1-{})",
                    workspace_num,
                    NUM_WORKSPACES
                );
                continue;
            }

            log::info!("Applying startup layout to workspace {}", workspace_num);

            // Build the layout tree from config
            let pending_apps = workspaces[ws_idx]
                .layout
                .replace_from_config(&ws_config.layout);

            // Collect spawns
            for (frame_id, commands) in pending_apps {
                let frame_name = workspaces[ws_idx]
                    .layout
                    .get_frame_name(frame_id)
                    .map(|s| s.to_string());

                for command in commands {
                    all_spawns.push(PendingSpawn {
                        command,
                        workspace_idx: ws_idx,
                        frame_id,
                        frame_name: frame_name.clone(),
                    });
                }
            }
        }

        self.pending_spawns = all_spawns.clone();
        all_spawns
    }

    /// Spawn all pending apps at once
    pub fn spawn_all(&mut self) {
        for spawn in self.pending_spawns.drain(..) {
            Self::spawn_command(&spawn.command, spawn.frame_name.as_deref());
        }
        self.startup_complete = true;
    }

    /// Spawn a single command
    fn spawn_command(command: &str, frame_name: Option<&str>) {
        let frame_info = frame_name
            .map(|n| format!(" in frame '{}'", n))
            .unwrap_or_default();
        log::info!("Startup: spawning '{}'{}", command, frame_info);

        // Handle shell expansion for paths like ~/projects
        let expanded = shellexpand::tilde(command);
        let parts: Vec<&str> = expanded.split_whitespace().collect();

        if let Some((program, args)) = parts.split_first() {
            let mut cmd = Command::new(program);
            cmd.args(args);

            // Detach from ttwm's process group so apps survive if ttwm exits
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                unsafe {
                    cmd.pre_exec(|| {
                        // Create new session to detach from terminal
                        libc::setsid();
                        Ok(())
                    });
                }
            }

            if let Err(e) = cmd.spawn() {
                log::error!("Failed to spawn startup app '{}': {}", command, e);
            }
        }
    }

    /// Check if startup is complete
    #[allow(dead_code)]
    pub fn is_complete(&self) -> bool {
        self.startup_complete
    }

    /// Mark startup as complete
    #[allow(dead_code)]
    pub fn mark_complete(&mut self) {
        self.startup_complete = true;
    }
}

impl Default for StartupManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FrameConfig, LayoutNodeConfig, SplitConfig, SplitDirectionConfig, WorkspaceStartup};

    fn create_test_workspaces() -> [Workspace; NUM_WORKSPACES] {
        std::array::from_fn(|i| Workspace::new(i + 1))
    }

    #[test]
    fn test_startup_manager_new() {
        let manager = StartupManager::new();
        assert!(!manager.is_complete());
        assert!(manager.pending_spawns.is_empty());
    }

    #[test]
    fn test_apply_config_single_frame() {
        let mut manager = StartupManager::new();
        let mut workspaces = create_test_workspaces();

        let mut config = StartupConfig::default();
        config.workspace.insert(
            "1".to_string(),
            WorkspaceStartup {
                layout: LayoutNodeConfig::Frame(FrameConfig {
                    name: Some("main".to_string()),
                    vertical_tabs: false,
                    apps: vec!["alacritty".to_string()],
                }),
            },
        );

        let spawns = manager.apply_config(&config, &mut workspaces);

        assert_eq!(spawns.len(), 1);
        assert_eq!(spawns[0].command, "alacritty");
        assert_eq!(spawns[0].workspace_idx, 0);
        assert_eq!(spawns[0].frame_name, Some("main".to_string()));
    }

    #[test]
    fn test_apply_config_multiple_workspaces() {
        let mut manager = StartupManager::new();
        let mut workspaces = create_test_workspaces();

        let mut config = StartupConfig::default();
        config.workspace.insert(
            "1".to_string(),
            WorkspaceStartup {
                layout: LayoutNodeConfig::Frame(FrameConfig {
                    apps: vec!["app1".to_string()],
                    ..Default::default()
                }),
            },
        );
        config.workspace.insert(
            "2".to_string(),
            WorkspaceStartup {
                layout: LayoutNodeConfig::Frame(FrameConfig {
                    apps: vec!["app2".to_string()],
                    ..Default::default()
                }),
            },
        );

        let spawns = manager.apply_config(&config, &mut workspaces);

        assert_eq!(spawns.len(), 2);
        // Check both workspaces have their apps
        let ws_indices: Vec<_> = spawns.iter().map(|s| s.workspace_idx).collect();
        assert!(ws_indices.contains(&0));
        assert!(ws_indices.contains(&1));
    }

    #[test]
    fn test_apply_config_split_layout() {
        let mut manager = StartupManager::new();
        let mut workspaces = create_test_workspaces();

        let mut config = StartupConfig::default();
        config.workspace.insert(
            "1".to_string(),
            WorkspaceStartup {
                layout: LayoutNodeConfig::Split(SplitConfig {
                    direction: SplitDirectionConfig::Horizontal,
                    ratio: 0.6,
                    first: Box::new(LayoutNodeConfig::Frame(FrameConfig {
                        name: Some("left".to_string()),
                        apps: vec!["code".to_string()],
                        ..Default::default()
                    })),
                    second: Box::new(LayoutNodeConfig::Frame(FrameConfig {
                        name: Some("right".to_string()),
                        apps: vec!["firefox".to_string()],
                        ..Default::default()
                    })),
                }),
            },
        );

        let spawns = manager.apply_config(&config, &mut workspaces);

        assert_eq!(spawns.len(), 2);

        // Verify workspace layout was changed
        assert_eq!(workspaces[0].layout.all_frames().len(), 2);
    }

    #[test]
    fn test_apply_config_invalid_workspace() {
        let mut manager = StartupManager::new();
        let mut workspaces = create_test_workspaces();

        let mut config = StartupConfig::default();
        // Workspace 10 is invalid (only 1-9 allowed)
        config.workspace.insert(
            "10".to_string(),
            WorkspaceStartup {
                layout: LayoutNodeConfig::Frame(FrameConfig::default()),
            },
        );

        let spawns = manager.apply_config(&config, &mut workspaces);

        // Should have no spawns (invalid workspace ignored)
        assert!(spawns.is_empty());
    }

    #[test]
    fn test_apply_config_no_apps() {
        let mut manager = StartupManager::new();
        let mut workspaces = create_test_workspaces();

        let mut config = StartupConfig::default();
        config.workspace.insert(
            "1".to_string(),
            WorkspaceStartup {
                layout: LayoutNodeConfig::Frame(FrameConfig {
                    name: Some("empty".to_string()),
                    apps: vec![], // No apps
                    ..Default::default()
                }),
            },
        );

        let spawns = manager.apply_config(&config, &mut workspaces);

        // No apps to spawn, but layout should still be applied
        assert!(spawns.is_empty());
        assert_eq!(workspaces[0].layout.all_frames().len(), 1);
    }

    #[test]
    fn test_spawn_all_marks_complete() {
        let mut manager = StartupManager::new();
        assert!(!manager.is_complete());

        manager.spawn_all();

        assert!(manager.is_complete());
    }
}
