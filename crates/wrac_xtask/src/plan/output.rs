use crate::XtaskOutputLanguage;
use crate::targets::PluginTarget;

use super::TaskStatus;

pub(super) fn status_label(language: XtaskOutputLanguage, status: TaskStatus) -> &'static str {
    match (language, status) {
        (XtaskOutputLanguage::English, TaskStatus::Planned) => "planned",
        (XtaskOutputLanguage::Japanese, TaskStatus::Planned) => "未実行",
        (XtaskOutputLanguage::English, TaskStatus::Ok) => "ok",
        (XtaskOutputLanguage::Japanese, TaskStatus::Ok) => "成功",
        (XtaskOutputLanguage::English, TaskStatus::Failed) => "failed",
        (XtaskOutputLanguage::Japanese, TaskStatus::Failed) => "失敗",
        (XtaskOutputLanguage::English, TaskStatus::Skipped) => "skipped",
        (XtaskOutputLanguage::Japanese, TaskStatus::Skipped) => "スキップ",
    }
}

pub(super) fn plan_heading(language: XtaskOutputLanguage) -> &'static str {
    match language {
        XtaskOutputLanguage::English => "Plan",
        XtaskOutputLanguage::Japanese => "実行計画",
    }
}

pub(super) fn dependencies_heading(language: XtaskOutputLanguage) -> &'static str {
    match language {
        XtaskOutputLanguage::English => "Dependencies",
        XtaskOutputLanguage::Japanese => "依存関係",
    }
}

pub(super) fn execution_heading(language: XtaskOutputLanguage) -> &'static str {
    match language {
        XtaskOutputLanguage::English => "Execution",
        XtaskOutputLanguage::Japanese => "実行",
    }
}

pub(super) fn result_heading(language: XtaskOutputLanguage) -> &'static str {
    match language {
        XtaskOutputLanguage::English => "Result",
        XtaskOutputLanguage::Japanese => "結果",
    }
}

pub(super) fn completed_label(language: XtaskOutputLanguage) -> &'static str {
    match language {
        XtaskOutputLanguage::English => "completed",
        XtaskOutputLanguage::Japanese => "完了",
    }
}

pub(super) fn success_label(language: XtaskOutputLanguage) -> &'static str {
    match language {
        XtaskOutputLanguage::English => "ok",
        XtaskOutputLanguage::Japanese => "成功",
    }
}

pub(super) fn failed_label(language: XtaskOutputLanguage) -> &'static str {
    match language {
        XtaskOutputLanguage::English => "failed",
        XtaskOutputLanguage::Japanese => "失敗",
    }
}

pub(super) fn skipped_label(language: XtaskOutputLanguage) -> &'static str {
    match language {
        XtaskOutputLanguage::English => "skipped",
        XtaskOutputLanguage::Japanese => "スキップ",
    }
}

pub(super) fn dry_run_message(language: XtaskOutputLanguage) -> &'static str {
    match language {
        XtaskOutputLanguage::English => "Nothing was executed because --dry-run was set.",
        XtaskOutputLanguage::Japanese => "--dry-run が指定されているため、実行はスキップしました。",
    }
}

pub(super) fn skip_reason(language: XtaskOutputLanguage, failed_deps: &[String]) -> String {
    match language {
        XtaskOutputLanguage::English => {
            format!(
                "Reason: dependency failed or was skipped ({})",
                failed_deps.join(", ")
            )
        }
        XtaskOutputLanguage::Japanese => {
            format!(
                "理由: 依存タスクが失敗またはスキップされました ({})",
                failed_deps.join(", ")
            )
        }
    }
}

pub(super) fn uninstall_summary(
    language: XtaskOutputLanguage,
    target: PluginTarget,
    removed: usize,
    missing: usize,
    dry_run: bool,
) -> String {
    match (language, dry_run) {
        (XtaskOutputLanguage::English, true) => format!(
            "Summary: {} {} would be removed, {} not found",
            target.display(),
            removed,
            missing
        ),
        (XtaskOutputLanguage::English, false) => format!(
            "Summary: {} {} removed, {} not found",
            target.display(),
            removed,
            missing
        ),
        (XtaskOutputLanguage::Japanese, true) => format!(
            "内訳: {} {}、{}",
            target.display(),
            count_with_unit(removed, "件削除予定"),
            count_with_unit(missing, "件なし")
        ),
        (XtaskOutputLanguage::Japanese, false) => format!(
            "内訳: {} {}、{}",
            target.display(),
            count_with_unit(removed, "件削除"),
            count_with_unit(missing, "件なし")
        ),
    }
}

fn count_with_unit(count: usize, unit: &str) -> String {
    format!("{count}{unit}")
}
