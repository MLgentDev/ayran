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
