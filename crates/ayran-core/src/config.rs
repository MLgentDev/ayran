use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::diagnostic::Diagnostic;
use crate::harness::{Effort, Harness};
use crate::launch::HomeMode;

pub struct Sourced<T> {
    pub value: T,
    pub path: PathBuf,
}

#[derive(Default)]
pub struct HarnessSettings {
    pub model: Option<Sourced<String>>,
    pub effort: Option<Sourced<Effort>>,
    /// User-level choice of a separate, ayran-owned Harness home.
    pub home_mode: HomeMode,
}

pub struct Alias {
    pub harness: Harness,
    pub model: Option<String>,
    pub effort: Option<Effort>,
    pub description: Option<String>,
    pub plugins: Vec<String>,
    pub skills: Vec<String>,
    pub mcp: Vec<String>,
    pub profiles: Vec<String>,
    pub defaults: bool,
    pub disabled_plugins: Vec<String>,
    pub disabled_skills: Vec<String>,
    pub disabled_mcp: Vec<String>,
    pub disabled_profiles: Vec<String>,
}

pub enum PluginBinding {
    Native(String),
    Path(PathBuf),
    Absent,
}

#[derive(Default)]
pub struct Plugin {
    pub all: Option<PluginBinding>,
    pub claude: Option<PluginBinding>,
    pub codex: Option<PluginBinding>,
    pub copilot: Option<PluginBinding>,
    pub description: Option<String>,
    pub default: bool,
}

impl Plugin {
    pub fn binding(&self, harness: Harness) -> Option<&PluginBinding> {
        let specific = match harness {
            Harness::Claude => &self.claude,
            Harness::Codex => &self.codex,
            Harness::Copilot => &self.copilot,
        };
        specific.as_ref().or(self.all.as_ref())
    }
}

pub enum SkillBinding {
    Native(String),
    Path(PathBuf),
    Absent,
}

#[derive(Default)]
pub struct Skill {
    pub default: bool,
    pub all: Option<SkillBinding>,
    pub claude: Option<SkillBinding>,
    pub codex: Option<SkillBinding>,
    pub copilot: Option<SkillBinding>,
    pub description: Option<String>,
}

impl Skill {
    pub fn binding(&self, harness: Harness) -> Option<&SkillBinding> {
        let specific = match harness {
            Harness::Claude => &self.claude,
            Harness::Codex => &self.codex,
            Harness::Copilot => &self.copilot,
        };
        specific.as_ref().or(self.all.as_ref())
    }
}

#[derive(Default)]
pub struct Profile {
    pub plugins: Vec<String>,
    pub skills: Vec<String>,
    pub mcp: Vec<String>,
    pub profiles: Vec<String>,
    pub description: Option<String>,
    pub default: bool,
}

#[derive(Default)]
pub struct ConfigLayers {
    pub default_harness: Option<Sourced<Harness>>,
    pub claude: HarnessSettings,
    pub codex: HarnessSettings,
    pub copilot: HarnessSettings,
    pub aliases: BTreeMap<String, Alias>,
    pub plugins: BTreeMap<String, Sourced<Plugin>>,
    pub skills: BTreeMap<String, Sourced<Skill>>,
    pub mcp: BTreeMap<String, Sourced<crate::mcp::McpServer>>,
    pub profiles: BTreeMap<String, Sourced<Profile>>,
    pub disabled_plugins: BTreeMap<String, PathBuf>,
    pub disabled_skills: BTreeMap<String, PathBuf>,
    pub disabled_mcp: BTreeMap<String, PathBuf>,
    pub disabled_default_mcp: BTreeMap<String, PathBuf>,
    pub disabled_default_skills: BTreeMap<String, PathBuf>,
    pub disabled_profiles: BTreeMap<String, PathBuf>,
    /// The reset declared in this layer (or the nearest reset after merging).
    pub disable_defaults: Option<PathBuf>,
    /// Default names removed by a reset in a nearer layer, with its source.
    pub disabled_defaults: BTreeMap<String, PathBuf>,
    /// Default Profile names removed by a reset in a nearer layer.
    pub disabled_default_profiles: BTreeMap<String, PathBuf>,
}

impl ConfigLayers {
    pub fn load_user() -> Result<Self, Diagnostic> {
        Ok(Self::read(&user_config_path()?, true)?.unwrap_or_default())
    }

    pub fn load() -> Result<Self, Diagnostic> {
        let mut layers = Self::load_unmerged()?.into_iter();
        let mut merged = layers.next().unwrap_or_default();
        for layer in layers {
            merged.merge(layer);
        }
        merged.validate_profiles()?;
        Ok(merged)
    }

    /// Preserve every layer so completion can offer overridden model hints.
    pub fn load_for_completion() -> Result<Vec<Self>, Diagnostic> {
        Self::load_unmerged()
    }

    fn load_unmerged() -> Result<Vec<Self>, Diagnostic> {
        let user_path = user_config_path()?;
        let mut layers = vec![Self::read_layer(&user_path, true)?.unwrap_or_default()];
        for path in directory_config_paths(&user_path)? {
            if let Some(layer) = Self::read_layer(&path, false)? {
                layers.push(layer);
            }
        }
        Ok(layers)
    }

    fn merge(&mut self, nearer: Self) {
        if let Some(path) = &nearer.disable_defaults {
            for (name, plugin) in &self.plugins {
                if plugin.value.default {
                    self.disabled_defaults.insert(name.clone(), path.clone());
                }
            }
            for (name, skill) in &self.skills {
                if skill.value.default {
                    self.disabled_default_skills
                        .insert(name.clone(), path.clone());
                }
            }
            for (name, server) in &self.mcp {
                if server.value.default {
                    self.disabled_default_mcp.insert(name.clone(), path.clone());
                }
            }
            for (name, profile) in &self.profiles {
                if profile.value.default {
                    self.disabled_default_profiles
                        .insert(name.clone(), path.clone());
                }
            }
            self.disable_defaults = nearer.disable_defaults;
        }
        for name in nearer.plugins.keys() {
            self.disabled_defaults.remove(name);
        }
        self.disabled_plugins.extend(nearer.disabled_plugins);
        self.disabled_profiles.extend(nearer.disabled_profiles);
        self.plugins.extend(nearer.plugins);
        for name in nearer.skills.keys() {
            self.disabled_default_skills.remove(name);
        }
        self.disabled_skills.extend(nearer.disabled_skills);
        self.skills.extend(nearer.skills);
        for name in nearer.mcp.keys() {
            self.disabled_default_mcp.remove(name);
        }
        self.disabled_mcp.extend(nearer.disabled_mcp);
        self.mcp.extend(nearer.mcp);
        for name in nearer.profiles.keys() {
            self.disabled_default_profiles.remove(name);
        }
        self.profiles.extend(nearer.profiles);
        if nearer.default_harness.is_some() {
            self.default_harness = nearer.default_harness;
        }
        for (farther, nearer) in [
            (&mut self.claude, nearer.claude),
            (&mut self.codex, nearer.codex),
            (&mut self.copilot, nearer.copilot),
        ] {
            if nearer.model.is_some() {
                farther.model = nearer.model;
            }
            if nearer.effort.is_some() {
                farther.effort = nearer.effort;
            }
        }
    }

    /// Validate and read one Config layer without loading surrounding layers.
    pub fn read(path: &Path, is_user: bool) -> Result<Option<Self>, Diagnostic> {
        Self::read_layer(path, is_user)
    }

    fn read_layer(path: &Path, is_user: bool) -> Result<Option<Self>, Diagnostic> {
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => return Err(invalid(path, error)),
        };
        let value: toml::Value = toml::from_str(&contents).map_err(|error| invalid(path, error))?;
        Self::parse_value(&value, path, is_user, &mut Vec::new(), false).map(Some)
    }

    fn parse_value(
        value: &toml::Value,
        path: &Path,
        is_user: bool,
        diagnostics: &mut Vec<Diagnostic>,
        collect: bool,
    ) -> Result<Self, Diagnostic> {
        let table = value
            .as_table()
            .ok_or_else(|| invalid(path, "expected a table"))?;
        let mut layers = Self::default();
        for (key, value) in table {
            match key.as_str() {
                "default_harness" => {
                    layers.default_harness = Some(Sourced {
                        value: parse_harness(value, path, "default_harness")?,
                        path: path.to_path_buf(),
                    });
                }
                "harnesses" => {
                    let harnesses = value
                        .as_table()
                        .ok_or_else(|| invalid(path, "harnesses must be a table"))?;
                    for (name, value) in harnesses {
                        let settings = match name.as_str() {
                            "claude" => &mut layers.claude,
                            "codex" => &mut layers.codex,
                            "copilot" => &mut layers.copilot,
                            _ => {
                                return Err(invalid(
                                    path,
                                    format!("unknown Harness table harnesses.{name}"),
                                ));
                            }
                        };
                        let fields = value.as_table().ok_or_else(|| {
                            invalid(path, format!("harnesses.{name} must be a table"))
                        })?;
                        for (field, value) in fields {
                            match field.as_str() {
                                "model" => {
                                    settings.model = Some(Sourced {
                                        value: value
                                            .as_str()
                                            .ok_or_else(|| {
                                                invalid(
                                                    path,
                                                    format!(
                                                        "harnesses.{name}.model must be a string"
                                                    ),
                                                )
                                            })?
                                            .to_owned(),
                                        path: path.to_path_buf(),
                                    })
                                }
                                "effort" => {
                                    settings.effort = Some(Sourced {
                                        value: parse_effort(value, path)?,
                                        path: path.to_path_buf(),
                                    })
                                }
                                "home" if !is_user => {
                                    report(user_level_only(path, "home"), diagnostics, collect)?;
                                }
                                "home" => {
                                    settings.home_mode = match value.as_str() {
                                        Some("isolated") => HomeMode::Isolated,
                                        Some("shared") => HomeMode::Shared,
                                        _ => {
                                            return Err(invalid(
                                                path,
                                                "home must be shared or isolated",
                                            ));
                                        }
                                    };
                                }
                                _ => {
                                    return Err(invalid(
                                        path,
                                        format!("unknown key harnesses.{name}.{field}"),
                                    ));
                                }
                            }
                        }
                    }
                }
                "aliases" => {
                    if !is_user {
                        report(user_level_only(path, "aliases"), diagnostics, collect)?;
                    }
                    let aliases = value
                        .as_table()
                        .ok_or_else(|| invalid(path, "aliases must be a table"))?;
                    for (name, value) in aliases {
                        if !valid_alias_name(name) {
                            report(
                                Diagnostic::error(
                                    "invalid-name",
                                    format!("{}: invalid Alias name {name}", path.display()),
                                    None,
                                ),
                                diagnostics,
                                collect,
                            )?;
                        }
                        let fields = value.as_table().ok_or_else(|| {
                            invalid(path, format!("aliases.{name} must be a table"))
                        })?;
                        let mut harness = None;
                        let mut model = None;
                        let mut effort = None;
                        let mut description = None;
                        let mut plugins = Vec::new();
                        let mut skills = Vec::new();
                        let mut mcp = Vec::new();
                        let mut profiles = Vec::new();
                        let mut defaults = true;
                        let mut disabled = Disable::default();
                        for (field, value) in fields {
                            match field.as_str() {
                                "harness" => {
                                    harness = Some(parse_harness(
                                        value,
                                        path,
                                        &format!("aliases.{name}.harness"),
                                    )?)
                                }
                                "model" => {
                                    model = Some(
                                        value
                                            .as_str()
                                            .ok_or_else(|| {
                                                invalid(
                                                    path,
                                                    format!(
                                                        "aliases.{name}.model must be a string"
                                                    ),
                                                )
                                            })?
                                            .to_owned(),
                                    )
                                }
                                "effort" => effort = Some(parse_effort(value, path)?),
                                "description" => description = Some(
                                    value
                                        .as_str()
                                        .ok_or_else(|| {
                                            invalid(
                                                path,
                                                format!(
                                                    "aliases.{name}.description must be a string"
                                                ),
                                            )
                                        })?
                                        .to_owned(),
                                ),
                                "plugins" => {
                                    plugins = parse_names(
                                        value,
                                        path,
                                        &format!("aliases.{name}.plugins"),
                                    )?
                                }
                                "profiles" => {
                                    profiles = parse_names(
                                        value,
                                        path,
                                        &format!("aliases.{name}.profiles"),
                                    )?
                                }
                                "defaults" => {
                                    defaults = parse_bool(
                                        value,
                                        path,
                                        &format!("aliases.{name}.defaults"),
                                    )?
                                }
                                "disable" => {
                                    disabled = parse_disable(
                                        value,
                                        path,
                                        &format!("aliases.{name}.disable"),
                                        false,
                                    )?
                                }
                                "skills" => {
                                    skills =
                                        parse_names(value, path, &format!("aliases.{name}.skills"))?
                                }
                                "mcp" => {
                                    mcp = parse_names(value, path, &format!("aliases.{name}.mcp"))?
                                }
                                _ => {
                                    return Err(invalid(
                                        path,
                                        format!("unknown key aliases.{name}.{field}"),
                                    ));
                                }
                            }
                        }
                        let harness = harness.ok_or_else(|| {
                            invalid(path, format!("aliases.{name}.harness is required"))
                        })?;
                        layers.aliases.insert(
                            name.clone(),
                            Alias {
                                harness,
                                model,
                                effort,
                                description,
                                plugins,
                                skills,
                                mcp,
                                profiles,
                                defaults,
                                disabled_plugins: disabled.plugins,
                                disabled_skills: disabled.skills,
                                disabled_mcp: disabled.mcp,
                                disabled_profiles: disabled.profiles,
                            },
                        );
                    }
                }
                "plugins" => {
                    let plugins = value
                        .as_table()
                        .ok_or_else(|| invalid(path, "plugins must be a table"))?;
                    for (name, value) in plugins {
                        if name.contains(',') {
                            report(
                                Diagnostic::error(
                                    "invalid-name",
                                    format!("{}: invalid Plugin name {name}", path.display()),
                                    None,
                                ),
                                diagnostics,
                                collect,
                            )?;
                        }
                        let fields = value.as_table().ok_or_else(|| {
                            invalid(path, format!("plugins.{name} must be a table"))
                        })?;
                        let mut plugin = Plugin::default();
                        for (field, value) in fields {
                            match field.as_str() {
                                "all" | "claude" | "codex" | "copilot" => {
                                    let binding = parse_plugin_binding(
                                        value,
                                        path,
                                        &format!("plugins.{name}.{field}"),
                                    )?;
                                    let slot = match field.as_str() {
                                        "all" => &mut plugin.all,
                                        "claude" => &mut plugin.claude,
                                        "codex" => &mut plugin.codex,
                                        "copilot" => &mut plugin.copilot,
                                        _ => unreachable!(),
                                    };
                                    *slot = Some(binding);
                                }
                                "description" => plugin.description = Some(
                                    value
                                        .as_str()
                                        .ok_or_else(|| {
                                            invalid(
                                                path,
                                                format!(
                                                    "plugins.{name}.description must be a string"
                                                ),
                                            )
                                        })?
                                        .to_owned(),
                                ),
                                "default" => {
                                    plugin.default = parse_bool(
                                        value,
                                        path,
                                        &format!("plugins.{name}.default"),
                                    )?;
                                }
                                _ => {
                                    return Err(invalid(
                                        path,
                                        format!("unknown key plugins.{name}.{field}"),
                                    ));
                                }
                            }
                        }
                        layers.plugins.insert(
                            name.clone(),
                            Sourced {
                                value: plugin,
                                path: path.to_path_buf(),
                            },
                        );
                    }
                }
                "disable" => {
                    let disabled = parse_disable(value, path, "disable", true)?;
                    layers.disabled_plugins = disabled
                        .plugins
                        .into_iter()
                        .map(|name| (name, path.to_path_buf()))
                        .collect();
                    layers.disabled_mcp = disabled
                        .mcp
                        .into_iter()
                        .map(|name| (name, path.to_path_buf()))
                        .collect();
                    layers.disabled_skills = disabled
                        .skills
                        .into_iter()
                        .map(|name| (name, path.to_path_buf()))
                        .collect();
                    layers.disabled_profiles = disabled
                        .profiles
                        .into_iter()
                        .map(|name| (name, path.to_path_buf()))
                        .collect();
                    if disabled.defaults {
                        layers.disable_defaults = Some(path.to_path_buf());
                    }
                }
                "profiles" => {
                    let profiles = value
                        .as_table()
                        .ok_or_else(|| invalid(path, "profiles must be a table"))?;
                    for (name, value) in profiles {
                        if name.contains(',') {
                            report(
                                Diagnostic::error(
                                    "invalid-name",
                                    format!("{}: invalid Profile name {name}", path.display()),
                                    None,
                                ),
                                diagnostics,
                                collect,
                            )?;
                        }
                        let fields = value.as_table().ok_or_else(|| {
                            invalid(path, format!("profiles.{name} must be a table"))
                        })?;
                        let mut profile = Profile::default();
                        for (field, value) in fields {
                            let key = format!("profiles.{name}.{field}");
                            match field.as_str() {
                                "plugins" => profile.plugins = parse_names(value, path, &key)?,
                                "profiles" => profile.profiles = parse_names(value, path, &key)?,
                                "default" => profile.default = parse_bool(value, path, &key)?,
                                "description" => {
                                    profile.description = Some(
                                        value
                                            .as_str()
                                            .ok_or_else(|| {
                                                invalid(path, format!("{key} must be a string"))
                                            })?
                                            .to_owned(),
                                    )
                                }
                                "skills" => profile.skills = parse_names(value, path, &key)?,
                                "mcp" => profile.mcp = parse_names(value, path, &key)?,
                                _ => return Err(invalid(path, format!("unknown key {key}"))),
                            }
                        }
                        layers.profiles.insert(
                            name.clone(),
                            Sourced {
                                value: profile,
                                path: path.to_path_buf(),
                            },
                        );
                    }
                }
                "skills" => {
                    let skills = value
                        .as_table()
                        .ok_or_else(|| invalid(path, "skills must be a table"))?;
                    for (name, value) in skills {
                        if name.contains(',') {
                            report(
                                Diagnostic::error(
                                    "invalid-name",
                                    format!("{}: invalid Skill name {name}", path.display()),
                                    None,
                                ),
                                diagnostics,
                                collect,
                            )?;
                        }
                        let fields = value.as_table().ok_or_else(|| {
                            invalid(path, format!("skills.{name} must be a table"))
                        })?;
                        let mut skill = Skill::default();
                        for (field, value) in fields {
                            let key = format!("skills.{name}.{field}");
                            match field.as_str() {
                                "all" | "claude" | "codex" | "copilot" => {
                                    let binding = match parse_plugin_binding(value, path, &key)? {
                                        PluginBinding::Native(id) => {
                                            if id.contains(':') {
                                                return Err(invalid(
                                                    path,
                                                    format!(
                                                        "{key}: native Skill Bindings cannot contain ':'"
                                                    ),
                                                ));
                                            }
                                            SkillBinding::Native(id)
                                        }
                                        PluginBinding::Path(path) => SkillBinding::Path(path),
                                        PluginBinding::Absent => SkillBinding::Absent,
                                    };
                                    let slot = match field.as_str() {
                                        "all" => &mut skill.all,
                                        "claude" => &mut skill.claude,
                                        "codex" => &mut skill.codex,
                                        "copilot" => &mut skill.copilot,
                                        _ => unreachable!(),
                                    };
                                    *slot = Some(binding);
                                }
                                "description" => {
                                    skill.description = Some(
                                        value
                                            .as_str()
                                            .ok_or_else(|| {
                                                invalid(path, format!("{key} must be a string"))
                                            })?
                                            .to_owned(),
                                    )
                                }
                                "default" => skill.default = parse_bool(value, path, &key)?,
                                _ => return Err(invalid(path, format!("unknown key {key}"))),
                            }
                        }
                        layers.skills.insert(
                            name.clone(),
                            Sourced {
                                value: skill,
                                path: path.to_path_buf(),
                            },
                        );
                    }
                }
                "mcp" => layers.mcp = crate::mcp::parse(value, path, diagnostics, collect)?,
                "home" if !is_user => report(user_level_only(path, "home"), diagnostics, collect)?,
                "home" => return Err(unsupported(path, "home")),
                _ => return Err(invalid(path, format!("unknown key {key}"))),
            }
        }
        if !is_user {
            layers.aliases.clear();
        }
        Ok(layers)
    }

    /// Load every layer for an audit, retaining semantic errors and all invalid layers.
    pub fn load_for_doctor() -> (Self, Vec<Diagnostic>) {
        let mut diagnostics = Vec::new();
        let paths = user_config_path().and_then(|user| {
            let mut paths = vec![(user.clone(), true)];
            paths.extend(
                directory_config_paths(&user)?
                    .into_iter()
                    .map(|p| (p, false)),
            );
            Ok(paths)
        });
        let paths = match paths {
            Ok(paths) => paths,
            Err(d) => return (Self::default(), vec![d]),
        };
        let mut merged = Self::default();
        for (path, is_user) in paths {
            let contents = match fs::read_to_string(&path) {
                Ok(contents) => contents,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    diagnostics.push(invalid(&path, e));
                    continue;
                }
            };
            let start = diagnostics.len();
            let parsed = toml::from_str::<toml::Value>(&contents)
                .map_err(|e| invalid(&path, e))
                .and_then(|value| {
                    Self::parse_value(&value, &path, is_user, &mut diagnostics, true)
                });
            match parsed {
                Ok(layer) if is_user => merged = layer,
                Ok(layer) => merged.merge(layer),
                Err(d) => diagnostics.push(d),
            }
            for diagnostic in &mut diagnostics[start..] {
                diagnostic.layer = Some(path.display().to_string());
            }
        }
        if diagnostics.iter().any(|d| d.code == "config-invalid") {
            diagnostics.retain(|d| d.code == "config-invalid");
        }
        (merged, diagnostics)
    }

    /// Check the merged graph, including Profiles that were not selected.
    /// Missing members are checked later, when expansion reaches them.
    pub fn validate_profiles(&self) -> Result<(), Diagnostic> {
        fn visit<'a>(
            layers: &'a ConfigLayers,
            name: &'a str,
            active: &mut Vec<&'a str>,
            done: &mut std::collections::BTreeSet<&'a str>,
        ) -> Result<(), Diagnostic> {
            if active.contains(&name) {
                active.push(name);
                return Err(Diagnostic::error(
                    "profile-cycle",
                    format!("Profile cycle: {}", active.join(" → ")),
                    None,
                ));
            }
            if done.contains(name) {
                return Ok(());
            }
            if let Some(profile) = layers.profiles.get(name) {
                active.push(name);
                for member in &profile.value.profiles {
                    visit(layers, member, active, done)?;
                }
                active.pop();
            }
            done.insert(name);
            Ok(())
        }
        let mut done = std::collections::BTreeSet::new();
        for name in self.profiles.keys() {
            visit(self, name, &mut Vec::new(), &mut done)?;
        }
        Ok(())
    }

    pub fn settings(&self, harness: Harness) -> &HarnessSettings {
        match harness {
            Harness::Claude => &self.claude,
            Harness::Codex => &self.codex,
            Harness::Copilot => &self.copilot,
        }
    }
}

fn valid_alias_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Project and local paths in loading order, including missing files.
/// The user layer is excluded even when it also lies on the directory path.
pub fn directory_config_paths(user_path: &Path) -> Result<Vec<PathBuf>, Diagnostic> {
    let canonical_user = user_path.canonicalize().ok();
    let cwd = env::current_dir().map_err(|error| {
        Diagnostic::error(
            "config-invalid",
            format!("current directory: {error}"),
            None,
        )
    })?;
    let mut paths = Vec::new();
    for directory in cwd.ancestors().collect::<Vec<_>>().into_iter().rev() {
        for name in ["ayran.toml", "ayran.local.toml"] {
            let path = directory.join(name);
            if !canonical_user
                .as_ref()
                .is_some_and(|user| path.canonicalize().is_ok_and(|found| found == *user))
            {
                paths.push(path);
            }
        }
    }
    Ok(paths)
}

pub fn user_config_path() -> Result<PathBuf, Diagnostic> {
    if let Some(path) = env::var_os("AYRAN_CONFIG") {
        return Ok(expand_home(PathBuf::from(path)));
    }
    #[cfg(windows)]
    {
        let base = env::var_os("APPDATA")
            .ok_or_else(|| Diagnostic::error("config-invalid", "APPDATA is not set", None))?;
        return Ok(PathBuf::from(base).join("ayran/ayran.toml"));
    }
    #[cfg(not(windows))]
    {
        if let Some(base) = env::var_os("XDG_CONFIG_HOME") {
            return Ok(expand_home(PathBuf::from(base)).join("ayran/ayran.toml"));
        }
        let home = env::var_os("HOME")
            .ok_or_else(|| Diagnostic::error("config-invalid", "HOME is not set", None))?;
        Ok(PathBuf::from(home).join(".config/ayran/ayran.toml"))
    }
}

fn expand_home(path: PathBuf) -> PathBuf {
    if let Ok(rest) = path.strip_prefix("~")
        && let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE"))
    {
        return PathBuf::from(home).join(rest);
    }
    path
}

fn parse_harness(value: &toml::Value, path: &Path, field: &str) -> Result<Harness, Diagnostic> {
    match value.as_str() {
        Some("claude") => Ok(Harness::Claude),
        Some("codex") => Ok(Harness::Codex),
        Some("copilot") => Ok(Harness::Copilot),
        _ => Err(invalid(
            path,
            format!("{field} must be claude, codex, or copilot"),
        )),
    }
}

fn parse_effort(value: &toml::Value, path: &Path) -> Result<Effort, Diagnostic> {
    match value.as_str() {
        Some("low") => Ok(Effort::Low),
        Some("medium") => Ok(Effort::Medium),
        Some("high") => Ok(Effort::High),
        Some("xhigh") => Ok(Effort::Xhigh),
        Some("max") => Ok(Effort::Max),
        _ => Err(invalid(
            path,
            "effort must be low, medium, high, xhigh, or max",
        )),
    }
}

pub(crate) fn parse_bool(
    value: &toml::Value,
    path: &Path,
    field: &str,
) -> Result<bool, Diagnostic> {
    value
        .as_bool()
        .ok_or_else(|| invalid(path, format!("{field} must be a boolean")))
}

pub(crate) fn parse_names(
    value: &toml::Value,
    path: &Path,
    field: &str,
) -> Result<Vec<String>, Diagnostic> {
    let names = value
        .as_array()
        .ok_or_else(|| invalid(path, format!("{field} must be an array of strings")))?;
    names
        .iter()
        .map(|name| {
            name.as_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid(path, format!("{field} must be an array of strings")))
        })
        .collect()
}

#[derive(Default)]
struct Disable {
    plugins: Vec<String>,
    skills: Vec<String>,
    mcp: Vec<String>,
    profiles: Vec<String>,
    defaults: bool,
}

fn parse_disable(
    value: &toml::Value,
    path: &Path,
    field: &str,
    allow_defaults: bool,
) -> Result<Disable, Diagnostic> {
    let fields = value
        .as_table()
        .ok_or_else(|| invalid(path, format!("{field} must be a table")))?;
    let mut disabled = Disable::default();
    for (key, value) in fields {
        let field = format!("{field}.{key}");
        match key.as_str() {
            "plugins" => disabled.plugins = parse_names(value, path, &field)?,
            "profiles" => disabled.profiles = parse_names(value, path, &field)?,
            "defaults" if allow_defaults => disabled.defaults = parse_bool(value, path, &field)?,
            "skills" => disabled.skills = parse_names(value, path, &field)?,
            "mcp" => disabled.mcp = parse_names(value, path, &field)?,
            _ => return Err(invalid(path, format!("unknown key {field}"))),
        }
    }
    Ok(disabled)
}

pub(crate) fn invalid(path: &Path, message: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::error(
        "config-invalid",
        format!("{}: {message}", path.display()),
        None,
    )
}

fn unsupported(path: &Path, key: &str) -> Diagnostic {
    invalid(path, format!("{key} is not supported yet"))
}

fn user_level_only(path: &Path, key: &str) -> Diagnostic {
    Diagnostic::error(
        "user-level-only",
        format!(
            "{}: {key} is only allowed in user-level config",
            path.display()
        ),
        None,
    )
}

fn parse_plugin_binding(
    value: &toml::Value,
    path: &Path,
    field: &str,
) -> Result<PluginBinding, Diagnostic> {
    match value {
        toml::Value::String(id) => Ok(PluginBinding::Native(id.clone())),
        toml::Value::Boolean(false) => Ok(PluginBinding::Absent),
        toml::Value::Table(fields) if fields.len() == 1 && fields.contains_key("path") => {
            let value = fields["path"]
                .as_str()
                .ok_or_else(|| invalid(path, format!("{field}.path must be a string")))?;
            let mut directory = expand_home(PathBuf::from(value));
            if !directory.is_absolute() {
                directory = path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(directory);
            }
            if !directory.is_absolute() {
                directory = env::current_dir()
                    .map_err(|error| invalid(path, error))?
                    .join(directory);
            }
            Ok(PluginBinding::Path(directory))
        }
        _ => Err(invalid(
            path,
            format!("{field} must be a native string, {{ path = \"…\" }}, or false"),
        )),
    }
}

pub(crate) fn report(
    diagnostic: Diagnostic,
    diagnostics: &mut Vec<Diagnostic>,
    collect: bool,
) -> Result<(), Diagnostic> {
    if collect {
        diagnostics.push(diagnostic);
        Ok(())
    } else {
        Err(diagnostic)
    }
}
