
use std::process::Command;
use std::io::Write;
use tempfile::NamedTempFile;
use anyhow::{Result, anyhow};
use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;

// Simplified structure mirroring the DB Skill node
pub struct Skill {
    pub name: String,
    pub command_name: String,
    pub script_content: Option<String>,
}

pub struct SkillExecutor;

impl SkillExecutor {
    pub fn new() -> Self {
        Self
    }

    pub fn execute(&self, skill: &Skill, args: &str) -> Result<String> {
        let mut cmd;

        if let Some(content) = &skill.script_content {
            // Write script to temp file
            let mut temp_file = NamedTempFile::new()?;
            write!(temp_file, "{}", content)?;
            
            // Make executable
            let path = temp_file.path().to_path_buf();
            std::fs::set_permissions(&path, Permissions::from_mode(0o755))?;

            // Execute (assuming bash/sh or dependent on shebang)
            // Ideally we detect the interpreter or rely on shebang
            cmd = Command::new(path);
        } else {
            // Execute binary from PATH
            cmd = Command::new(&skill.command_name);
        }

        // Add arguments
        // Simple splitting by space is dangerous for quoted args.
        // For now, we assume args are space-separated or handled by the script.
        // Better: use shlex or similar crate.
        // Fallback: pass as single arg if possible or rely on shell?
        // Let's split by whitespace for simple V1.
        for arg in args.split_whitespace() {
            cmd.arg(arg);
        }

        let output = cmd.output().map_err(|e| anyhow!("Failed to execute skill '{}': {}", skill.name, e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if output.status.success() {
            Ok(stdout.to_string())
        } else {
             Err(anyhow!("Skill execution failed: {}\nStderr: {}", stdout, stderr))
        }
    }
}
