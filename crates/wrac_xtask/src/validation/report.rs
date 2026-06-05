use std::fmt::Write as _;

use super::checks::{CheckResult, CheckStatus, RuleViolation};

pub(crate) fn print_check_results(results: &[CheckResult]) {
    println!("WRAC production-readiness checks:");
    for result in results {
        match &result.status {
            CheckStatus::Passed => println!("  pass     {}", result.rule_id),
            CheckStatus::Skipped(reason) => {
                println!("  skipped  {}", result.rule_id);
                println!("           reason: {reason}");
            }
            CheckStatus::Disabled(reason) => {
                println!("  disabled {}", result.rule_id);
                println!("           reason: {reason}");
            }
            CheckStatus::Failed(violations) => {
                println!("  fail     {}", result.rule_id);
                for violation in violations {
                    println!("           {}", violation.message);
                    println!("           Fix: {}", violation.fix);
                }
            }
        }
    }
}

pub(crate) fn failed_violations(results: &[CheckResult]) -> Vec<&RuleViolation> {
    // Reporting and process failure are intentionally separate: CI should display the full
    // check matrix, while the command's non-zero exit is determined only by failed checks.
    results
        .iter()
        .flat_map(|result| match &result.status {
            CheckStatus::Failed(violations) => violations.iter().collect::<Vec<_>>(),
            CheckStatus::Passed | CheckStatus::Skipped(_) | CheckStatus::Disabled(_) => Vec::new(),
        })
        .collect()
}

pub(crate) fn failure_message(violations: &[&RuleViolation]) -> String {
    let mut message = String::from("WRAC production-readiness checks failed:\n");
    for violation in violations {
        let _ = writeln!(
            message,
            "\n{}:\n  error {}\n    {}\n    Fix: {}",
            violation.location.display(),
            violation.rule_id,
            violation.message,
            violation.fix
        );
    }
    message
}
