use std::fmt::Write as _;

use crate::XtaskOutputLanguage;

use super::checks::{CheckResult, CheckStatus, RuleViolation};

pub(crate) fn print_check_results(results: &[CheckResult], language: XtaskOutputLanguage) {
    println!();
    println!(
        "== {} ==",
        match language {
            XtaskOutputLanguage::English => "WRAC production-readiness checks",
            XtaskOutputLanguage::Japanese => "WRAC production-readiness check",
        }
    );
    println!();
    for result in results {
        let product = product_label(&result.plugin_name, &result.plugin_id);
        match &result.status {
            CheckStatus::Passed => println!(
                "  ✅ {} {} [{}]",
                status_label(language, "pass", "成功"),
                result.rule_id,
                product
            ),
            CheckStatus::Skipped(reason) => {
                println!(
                    "  ⏭️ {} {} [{}]",
                    status_label(language, "skipped", "スキップ"),
                    result.rule_id,
                    product
                );
                println!("     {}: {reason}", reason_label(language));
            }
            CheckStatus::Disabled(reason) => {
                println!(
                    "  ⏭️ {} {} [{}]",
                    status_label(language, "disabled", "無効"),
                    result.rule_id,
                    product
                );
                println!("     {}: {reason}", reason_label(language));
            }
            CheckStatus::Failed(violations) => {
                println!(
                    "  ❌ {} {} [{}]",
                    status_label(language, "fail", "失敗"),
                    result.rule_id,
                    product
                );
                for violation in violations {
                    println!("     {}", violation.message);
                    println!("     {}: {}", fix_label(language), violation.fix);
                }
            }
        }
    }
}

fn status_label(
    language: XtaskOutputLanguage,
    english: &'static str,
    japanese: &'static str,
) -> &'static str {
    match language {
        XtaskOutputLanguage::English => english,
        XtaskOutputLanguage::Japanese => japanese,
    }
}

fn reason_label(language: XtaskOutputLanguage) -> &'static str {
    match language {
        XtaskOutputLanguage::English => "reason",
        XtaskOutputLanguage::Japanese => "理由",
    }
}

fn fix_label(language: XtaskOutputLanguage) -> &'static str {
    match language {
        XtaskOutputLanguage::English => "Fix",
        XtaskOutputLanguage::Japanese => "修正",
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

pub(crate) fn failure_message(
    violations: &[&RuleViolation],
    language: XtaskOutputLanguage,
) -> String {
    let mut message = match language {
        XtaskOutputLanguage::English => String::from("WRAC production-readiness checks failed:\n"),
        XtaskOutputLanguage::Japanese => {
            String::from("WRAC production-readiness check に失敗しました:\n")
        }
    };
    for violation in violations {
        let product = product_label(&violation.plugin_name, &violation.plugin_id);
        match language {
            XtaskOutputLanguage::English => {
                let _ = writeln!(
                    message,
                    "\n{}:\n  product {}\n  error {}\n    {}\n    Fix: {}",
                    violation.location.display(),
                    product,
                    violation.rule_id,
                    violation.message,
                    violation.fix
                );
            }
            XtaskOutputLanguage::Japanese => {
                let _ = writeln!(
                    message,
                    "\n{}:\n  product {}\n  error {}\n    {}\n    修正: {}",
                    violation.location.display(),
                    product,
                    violation.rule_id,
                    violation.message,
                    violation.fix
                );
            }
        }
    }
    message
}

fn product_label(plugin_name: &str, plugin_id: &str) -> String {
    if plugin_name.is_empty() {
        plugin_id.to_string()
    } else {
        format!("{plugin_name} ({plugin_id})")
    }
}
