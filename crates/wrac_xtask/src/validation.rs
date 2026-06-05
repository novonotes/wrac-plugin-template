mod checks;
mod clap_schema;
mod report;

use crate::Result;
use crate::context::Context;
use crate::profile::BuildProfile;
use crate::targets::ValidateTarget;

pub(crate) fn validate_wrac_rules(
    ctx: &Context,
    profile: BuildProfile,
    targets: &[ValidateTarget],
) -> Result<()> {
    checks::validate_disabled_rules(&ctx.metadata.validation)?;

    // Run these checks before external validators so release-policy failures are visible even
    // when a format validator would also reject the artifact later.
    let clap = ctx.clap_bundle(profile);
    let schemas = unsafe { clap_schema::read_clap_schemas(ctx, profile, &clap)? };
    let mut results = schemas
        .iter()
        .flat_map(|schema| {
            checks::evaluate_checks(
                schema,
                targets,
                &ctx.metadata.validation,
                &ctx.plugin_manifest(),
            )
        })
        .collect::<Vec<_>>();
    results.extend(checks::evaluate_source_checks(
        &ctx.metadata,
        &ctx.metadata.validation,
        &ctx.plugin_manifest(),
        &ctx.root,
    ));

    // Print the full matrix first. CI logs need to show checks that passed, were disabled,
    // or were skipped; the final error only contains checks that failed.
    report::print_check_results(&results);
    let violations = report::failed_violations(&results);
    if violations.is_empty() {
        println!("WRAC production-readiness checks: passed");
        return Ok(());
    }

    Err(report::failure_message(&violations).into())
}
