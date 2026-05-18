use crate::wutil::wgetcwd;
use fish_widestring::{WString, wcs2bytes};
use std::{
    ffi::OsStr,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

pub(crate) struct ShellContext {
    system: Option<SystemContext>,
    cwd: Option<WString>,
    git: Option<GitContext>,
}

struct ContextConfig {
    system: bool,
    cwd: bool,
    git: bool,
}

struct SystemContext {
    os: &'static str,
    arch: &'static str,
}

struct GitContext {
    branch: Option<String>,
    staged_count: usize,
    unstaged_count: usize,
    untracked_count: usize,
}

impl ShellContext {
    pub(crate) fn capture() -> Self {
        let config = ContextConfig::from_env();
        let cwd = wgetcwd();
        let git = config.git.then(|| git_context(&cwd)).flatten();
        Self {
            system: config.system.then_some(SystemContext {
                os: std::env::consts::OS,
                arch: std::env::consts::ARCH,
            }),
            cwd: (config.cwd && !cwd.is_empty()).then_some(cwd),
            git,
        }
    }

    pub(crate) fn as_prompt_section(&self) -> String {
        let mut lines = vec!["Environment:".to_owned()];
        if let Some(system) = &self.system {
            lines.push(format!("- OS: {}", system.os));
            lines.push(format!("- Arch: {}", system.arch));
        }
        if let Some(cwd) = &self.cwd {
            lines.push(format!("- Current directory: {}", wide_to_string(cwd)));
        }
        if let Some(git) = &self.git {
            let branch = git.branch.as_deref().unwrap_or("unknown");
            let state = if git.is_clean() {
                "clean".to_owned()
            } else {
                format!(
                    "{} staged, {} unstaged, {} untracked",
                    git.staged_count, git.unstaged_count, git.untracked_count
                )
            };
            lines.push(format!("- Git: branch {branch}, {state}"));
        }
        if lines.len() == 1 {
            return String::new();
        }
        lines.join("\n")
    }
}

impl ContextConfig {
    fn from_env() -> Self {
        Self {
            system: env_flag_enabled("ANGLER_AI_CONTEXT_SYSTEM", true),
            cwd: env_flag_enabled("ANGLER_AI_CONTEXT_CWD", true),
            git: env_flag_enabled("ANGLER_AI_CONTEXT_GIT", false),
        }
    }
}

impl GitContext {
    fn is_clean(&self) -> bool {
        self.staged_count == 0 && self.unstaged_count == 0 && self.untracked_count == 0
    }
}

fn git_context(cwd: &WString) -> Option<GitContext> {
    if cwd.is_empty() {
        return None;
    }
    let cwd = wide_to_string(cwd);
    if run_git(&cwd, &["rev-parse", "--is-inside-work-tree"])?.trim() != "true" {
        return None;
    }
    let branch = run_git(&cwd, &["branch", "--show-current"])
        .map(|branch| branch.trim().to_owned())
        .filter(|branch| !branch.is_empty())
        .or_else(|| {
            run_git(&cwd, &["rev-parse", "--short", "HEAD"])
                .map(|head| format!("detached@{}", head.trim()))
                .filter(|head| head != "detached@")
        });
    let status = run_git(&cwd, &["status", "--porcelain"])?;
    let mut context = GitContext {
        branch,
        staged_count: 0,
        unstaged_count: 0,
        untracked_count: 0,
    };
    for line in status.lines() {
        let bytes = line.as_bytes();
        if bytes.starts_with(b"??") {
            context.untracked_count += 1;
            continue;
        }
        if bytes.first().is_some_and(|status| *status != b' ') {
            context.staged_count += 1;
        }
        if bytes.get(1).is_some_and(|status| *status != b' ') {
            context.unstaged_count += 1;
        }
    }
    Some(context)
}

fn wide_to_string(value: &WString) -> String {
    String::from_utf8_lossy(&wcs2bytes(value)).into_owned()
}

fn env_flag_enabled(name: &str, default: bool) -> bool {
    let value = std::env::var_os(name);
    env_flag_value_enabled(value.as_deref(), default)
}

fn env_flag_value_enabled(value: Option<&OsStr>, default: bool) -> bool {
    let Some(value) = value else {
        return default;
    };
    let value = value.to_string_lossy();
    if value.is_empty() {
        return default;
    }
    !matches!(
        value.as_ref(),
        "0" | "false" | "FALSE" | "no" | "NO" | "off" | "OFF"
    )
}

fn run_git(cwd: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(100) {
        match child.try_wait().ok()? {
            Some(status) => {
                if !status.success() {
                    return None;
                }
                let output = child.wait_with_output().ok()?;
                return String::from_utf8(output.stdout).ok();
            }
            None => thread::sleep(Duration::from_millis(5)),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_section_includes_core_context() {
        let context = ShellContext {
            system: Some(SystemContext {
                os: "macos",
                arch: "aarch64",
            }),
            cwd: Some(WString::from_str("/tmp/project")),
            git: Some(GitContext {
                branch: Some("main".to_owned()),
                staged_count: 1,
                unstaged_count: 2,
                untracked_count: 3,
            }),
        };
        let prompt = context.as_prompt_section();
        assert!(prompt.contains("- OS: macos"));
        assert!(prompt.contains("- Arch: aarch64"));
        assert!(prompt.contains("- Current directory: /tmp/project"));
        assert!(prompt.contains("- Git: branch main, 1 staged, 2 unstaged, 3 untracked"));
    }

    #[test]
    fn prompt_section_can_only_include_enabled_context() {
        let context = ShellContext {
            system: None,
            cwd: Some(WString::from_str("/tmp/project")),
            git: None,
        };
        let prompt = context.as_prompt_section();
        assert!(!prompt.contains("- OS:"));
        assert!(prompt.contains("- Current directory: /tmp/project"));
        assert!(!prompt.contains("- Git:"));
    }

    #[test]
    fn prompt_section_is_empty_without_context() {
        let context = ShellContext {
            system: None,
            cwd: None,
            git: None,
        };
        assert!(context.as_prompt_section().is_empty());
    }

    #[test]
    fn env_flag_value_parses_disabled_values() {
        assert!(!env_flag_value_enabled(Some(OsStr::new("0")), true));
        assert!(!env_flag_value_enabled(Some(OsStr::new("false")), true));
        assert!(env_flag_value_enabled(Some(OsStr::new("1")), false));
        assert!(env_flag_value_enabled(None, true));
    }
}
