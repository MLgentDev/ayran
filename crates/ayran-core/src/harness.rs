use std::ffi::OsString;

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

pub struct HarnessLaunchOptions {
    pub args: Vec<OsString>,
    pub env_remove: Vec<OsString>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    Claude,
    Codex,
    Copilot,
}

impl Harness {
    pub fn binary(self) -> &'static str {
        match self {
            Self::Claude => claude::BINARY,
            Self::Codex => codex::BINARY,
            Self::Copilot => copilot::BINARY,
        }
    }

    /// Inspect only the model and Effort options ayran itself manages.
    pub fn args_overlap_model_or_effort(self, args: &[String]) -> bool {
        let flag = |arg: &str, name: &str| {
            arg == name
                || arg
                    .strip_prefix(name)
                    .is_some_and(|rest| rest.starts_with('='))
        };
        args.iter().enumerate().any(|(index, arg)| match self {
            Self::Claude => flag(arg, "--model") || flag(arg, "--effort"),
            Self::Copilot => flag(arg, "--model") || flag(arg, "--reasoning-effort"),
            Self::Codex => {
                flag(arg, "-m")
                    || flag(arg, "--model")
                    || arg.starts_with("-m") && arg.len() > 2
                    || (arg == "-c" || arg == "--config")
                        && args.get(index + 1).is_some_and(|value| {
                            value
                                .split_once('=')
                                .is_some_and(|(key, _)| key.trim() == "model_reasoning_effort")
                        })
                    || ["-c", "--config="].iter().any(|prefix| {
                        arg.strip_prefix(prefix).is_some_and(|value| {
                            value
                                .split_once('=')
                                .is_some_and(|(key, _)| key.trim() == "model_reasoning_effort")
                        })
                    })
            }
        })
    }

    pub fn translate_model_and_effort(
        self,
        model: Option<&str>,
        effort: Option<Effort>,
    ) -> HarnessLaunchOptions {
        let mut args = Vec::new();
        let mut env_remove = Vec::new();
        if self == Self::Copilot {
            env_remove.push("COPILOT_SKILLS_DIRS".into());
        }
        if let Some(model) = model {
            args.push(
                match self {
                    Self::Codex => "-m",
                    Self::Claude | Self::Copilot => "--model",
                }
                .into(),
            );
            args.push(model.into());
        }
        if let Some(effort) = effort {
            match self {
                Self::Claude => {
                    args.extend([OsString::from("--effort"), effort.as_str().into()]);
                    env_remove.push("CLAUDE_CODE_EFFORT_LEVEL".into());
                }
                Self::Codex => {
                    args.extend([
                        OsString::from("-c"),
                        format!("model_reasoning_effort=\"{}\"", effort.as_str()).into(),
                    ]);
                }
                Self::Copilot => {
                    args.extend([OsString::from("--reasoning-effort"), effort.as_str().into()]);
                }
            }
        }
        HarnessLaunchOptions { args, env_remove }
    }
}

pub mod claude {
    pub const BINARY: &str = "claude";
}

pub mod codex {
    pub const BINARY: &str = "codex";
}

pub mod copilot {
    pub const BINARY: &str = "copilot";
}
