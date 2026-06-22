use std::collections::HashSet;

use crate::context::Context;
use crate::targets::{PluginFormat, PluginTarget, Target, ValidateTarget};
use crate::{Result, XtaskOutputLanguage};

pub fn resolve_build_targets_from_metadata(
    ctx: &Context,
    requested: &[Target],
) -> Result<Vec<Target>> {
    let mut targets = if requested.is_empty() {
        // supported_formats is the product policy. The development standalone
        // remains outside that list because it is not a plugin format. Default
        // selection is platform-aware so a product can support AU without making
        // Windows/Linux builds fail unless AU was explicitly requested.
        let mut targets = ctx
            .metadata
            .supported_formats
            .iter()
            .map(|format| format.target())
            .collect::<Vec<_>>();
        targets.push(Target::Standalone);
        filter_platform_targets(ctx, targets)
    } else {
        requested.to_vec()
    };
    targets = dedup(targets);
    validate_target_support(ctx, &targets, !requested.is_empty())?;
    Ok(targets)
}

pub fn resolve_plugin_targets_from_metadata(
    ctx: &Context,
    requested: &[PluginTarget],
) -> Result<Vec<PluginTarget>> {
    let targets = if requested.is_empty() {
        filter_platform_targets(
            ctx,
            ctx.metadata
                .supported_formats
                .iter()
                .map(|format| format.target())
                .collect::<Vec<_>>(),
        )
        .into_iter()
        .filter_map(|target| match target {
            Target::Clap => Some(PluginTarget::Clap),
            Target::Vst3 => Some(PluginTarget::Vst3),
            Target::Au => Some(PluginTarget::Au),
            Target::Aax => Some(PluginTarget::Aax),
            Target::Standalone => None,
        })
        .collect::<Vec<_>>()
    } else {
        requested.to_vec()
    };
    let targets = dedup(targets);
    validate_plugin_format_support(
        ctx,
        &plugin_formats_for_plugin_targets(&targets),
        !requested.is_empty(),
    )?;
    Ok(targets)
}

pub fn resolve_validate_targets_from_metadata(
    ctx: &Context,
    requested: &[ValidateTarget],
) -> Result<Vec<ValidateTarget>> {
    let targets = if requested.is_empty() {
        filter_platform_targets(
            ctx,
            ctx.metadata
                .supported_formats
                .iter()
                .map(|format| format.target())
                .collect::<Vec<_>>(),
        )
        .into_iter()
        .filter_map(|target| match target {
            Target::Clap => Some(ValidateTarget::Clap),
            Target::Vst3 => Some(ValidateTarget::Vst3),
            Target::Au => Some(ValidateTarget::Au),
            Target::Aax => Some(ValidateTarget::Aax),
            Target::Standalone => None,
        })
        .collect::<Vec<_>>()
    } else {
        requested.to_vec()
    };
    let targets = dedup(targets);
    validate_plugin_format_support(
        ctx,
        &plugin_formats_for_validate_targets(&targets),
        !requested.is_empty(),
    )?;
    Ok(targets)
}

fn filter_platform_targets(ctx: &Context, targets: Vec<Target>) -> Vec<Target> {
    targets
        .into_iter()
        .filter(|target| {
            let supported = ctx.platform.supports_target(*target);
            if !supported {
                match ctx.output_language {
                    XtaskOutputLanguage::English => println!(
                        "  ⏭️ Skipping {}: not supported on {}.",
                        target.display(),
                        ctx.platform.display()
                    ),
                    XtaskOutputLanguage::Japanese => println!(
                        "  ⏭️ スキップ {}: {} では未対応",
                        target.display(),
                        ctx.platform.display()
                    ),
                }
            }
            supported
        })
        .collect()
}

fn plugin_formats_for_targets(targets: &[Target]) -> Vec<PluginFormat> {
    targets
        .iter()
        .filter_map(|target| target.plugin_format())
        .collect()
}

fn plugin_formats_for_plugin_targets(targets: &[PluginTarget]) -> Vec<PluginFormat> {
    targets.iter().map(|target| target.format()).collect()
}

fn plugin_formats_for_validate_targets(targets: &[ValidateTarget]) -> Vec<PluginFormat> {
    targets.iter().map(|target| target.format()).collect()
}

fn validate_target_support(ctx: &Context, targets: &[Target], explicit: bool) -> Result<()> {
    validate_plugin_format_support(ctx, &plugin_formats_for_targets(targets), explicit)?;
    validate_platform_target_support(ctx, targets)
}

fn validate_platform_target_support(ctx: &Context, targets: &[Target]) -> Result<()> {
    for target in targets {
        if !ctx.platform.supports_target(*target) {
            return Err(format!(
                "{} is not supported on {}",
                target.display(),
                ctx.platform.display()
            )
            .into());
        }
    }
    Ok(())
}

fn validate_plugin_format_support(
    ctx: &Context,
    formats: &[PluginFormat],
    explicit: bool,
) -> Result<()> {
    let supported = ctx
        .metadata
        .supported_formats
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    for format in formats {
        // An explicit --target is a request, not a hint. If a package does not
        // advertise that format, fail instead of silently falling back to the
        // supported subset.
        if explicit && !supported.contains(format) {
            return Err(format!(
                "{} is not listed in bundle.supported_formats for {}",
                format.display(),
                ctx.package_name
            )
            .into());
        }
        if !ctx.platform.supports_target(format.target()) {
            return Err(format!(
                "{} is not supported on {}",
                format.display(),
                ctx.platform.display()
            )
            .into());
        }
    }
    Ok(())
}

fn dedup<T: Copy + Eq + std::hash::Hash>(values: Vec<T>) -> Vec<T> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(*value))
        .collect()
}
