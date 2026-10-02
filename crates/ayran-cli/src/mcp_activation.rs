//! Filesystem boundary for MCP command validation and generated config files.

use std::fs;

use ayran_core::cache::GeneratedMcp;
use ayran_core::diagnostic::Diagnostic;
use ayran_core::harness::Harness;
use ayran_core::launch::LaunchPlan;

pub fn prepare(plan: &mut LaunchPlan) -> Result<Option<GeneratedMcp>, Diagnostic> {
    for path in &plan.mcp.command_paths {
        if !path.is_file() {
            return Err(Diagnostic::error(
                "path-not-found",
                format!(
                    "MCP command {} does not exist or is not a file",
                    path.display()
                ),
                None,
            ));
        }
    }
    let Some(config) = &plan.mcp.config else {
        return Ok(None);
    };
    let root = crate::generated_cache::root().ok_or_else(|| {
        Diagnostic::error(
            "config-invalid",
            "cannot locate MCP cache without XDG_CACHE_HOME or HOME",
            None,
        )
    })?;
    let harness = if plan.program == Harness::Copilot.binary() {
        Harness::Copilot
    } else {
        Harness::Claude
    };
    let cache = GeneratedMcp::new(&root, harness, config);
    let (flag, file) = if harness == Harness::Copilot {
        let mut file = std::ffi::OsString::from("@");
        file.push(cache.file());
        ("--additional-mcp-config", file)
    } else {
        // Claude's variadic --mcp-config is bounded by existing model/effort/settings flags.
        ("--mcp-config", cache.file().into_os_string())
    };
    plan.args.splice(0..0, [flag.into(), file]);
    Ok(Some(cache))
}

pub fn materialize(cache: &GeneratedMcp) -> Result<(), Diagnostic> {
    crate::generated_cache::materialize(&cache.directory, |temporary| {
        fs::write(temporary.join("mcp.json"), &cache.contents)
    })
    .map_err(|error| {
        Diagnostic::error(
            "config-invalid",
            format!(
                "cannot prepare MCP cache {}: {error}",
                cache.directory.display()
            ),
            None,
        )
    })
}
