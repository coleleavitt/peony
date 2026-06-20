use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use peony_cache::{PartialRelinkFallback, PartialRelinkPlan};
use serde_json::{Value, json};

pub(crate) struct CacheReportSink {
    path: Option<PathBuf>,
    diagnostics: bool,
}

impl CacheReportSink {
    pub(crate) fn new(path: Option<PathBuf>, diagnostics: bool) -> Self {
        Self { path, diagnostics }
    }

    pub(crate) fn record(&self, output: &Path, outcome: CacheOutcome<'_>) -> Result<()> {
        if self.diagnostics {
            eprintln!("peony: incremental cache: {}", outcome.message());
        }
        let Some(path) = &self.path else {
            return Ok(());
        };
        write_json(path, &outcome.to_json(output))
    }
}

pub(crate) enum CacheOutcome<'a> {
    ReusedUnchanged,
    PartialRelink { plan: &'a PartialRelinkPlan },
    FullEmit { reason: FullEmitReason<'a> },
}

impl CacheOutcome<'_> {
    fn message(&self) -> String {
        match self {
            Self::ReusedUnchanged => "reused unchanged output".to_string(),
            Self::PartialRelink { plan } => format!(
                "partial relink used: {} red sections, {} green sections",
                plan.red_count(),
                plan.green_count()
            ),
            Self::FullEmit { reason } => format!("full emit fallback: {}", reason.message()),
        }
    }

    fn to_json(&self, output: &Path) -> Value {
        let base = json!({
            "version": 1,
            "output": output.display().to_string(),
        });
        match self {
            Self::ReusedUnchanged => merge(
                base,
                json!({
                    "cache": { "enabled": true },
                    "action": "reused_unchanged_output",
                    "message": self.message(),
                }),
            ),
            Self::PartialRelink { plan } => merge(
                base,
                json!({
                    "cache": { "enabled": true },
                    "action": "partial_relink",
                    "message": self.message(),
                    "sections": {
                        "red": sorted_sections(plan.red_sections()),
                        "green": sorted_sections(plan.green_sections()),
                    },
                }),
            ),
            Self::FullEmit { reason } => merge(
                base,
                json!({
                    "cache": { "enabled": reason.incremental_enabled() },
                    "action": "full_emit",
                    "message": self.message(),
                    "reason": reason.to_json(),
                }),
            ),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum FullEmitReason<'a> {
    IncrementalDisabled,
    CacheStateUnavailable,
    PlannerFallback(&'a PartialRelinkFallback),
    PartialEmitDeclined,
}

impl FullEmitReason<'_> {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::IncrementalDisabled => "incremental_disabled",
            Self::CacheStateUnavailable => "cache_state_unavailable",
            Self::PlannerFallback(reason) => reason.code(),
            Self::PartialEmitDeclined => "partial_emit_declined",
        }
    }

    fn message(self) -> String {
        match self {
            Self::IncrementalDisabled => "incremental cache disabled".to_string(),
            Self::CacheStateUnavailable => "no usable prior cache state".to_string(),
            Self::PlannerFallback(reason) => reason.message(),
            Self::PartialEmitDeclined => {
                "partial emit declined because section preservation was unsafe".to_string()
            }
        }
    }

    fn incremental_enabled(self) -> bool {
        !matches!(self, Self::IncrementalDisabled)
    }

    fn to_json(self) -> Value {
        match self {
            Self::PlannerFallback(reason) => fallback_json(reason),
            _ => json!({
                "code": self.code(),
                "message": self.message(),
            }),
        }
    }
}

fn fallback_json(reason: &PartialRelinkFallback) -> Value {
    let base = json!({
        "code": reason.code(),
        "message": reason.message(),
    });
    match reason {
        PartialRelinkFallback::MissingSectionMetadata => base,
        PartialRelinkFallback::MissingPreviousSection { section } => merge(
            base,
            json!({
                "section": section,
            }),
        ),
        PartialRelinkFallback::SectionFileOffsetChanged {
            section,
            previous,
            current,
        }
        | PartialRelinkFallback::SectionVirtualAddressChanged {
            section,
            previous,
            current,
        }
        | PartialRelinkFallback::SectionSizeChanged {
            section,
            previous,
            current,
        } => merge(
            base,
            json!({
                "section": section,
                "previous": previous,
                "current": current,
            }),
        ),
        PartialRelinkFallback::SectionCapacityExceeded {
            section,
            capacity,
            size,
        } => merge(
            base,
            json!({
                "section": section,
                "capacity": capacity,
                "size": size,
            }),
        ),
    }
}

fn sorted_sections(sections: &HashSet<String>) -> Vec<&str> {
    let mut out: Vec<&str> = sections.iter().map(String::as_str).collect();
    out.sort_unstable();
    out
}

fn merge(mut left: Value, right: Value) -> Value {
    let Some(left) = left.as_object_mut() else {
        return right;
    };
    let Some(right) = right.as_object() else {
        return Value::Object(std::mem::take(left));
    };
    for (key, value) in right {
        left.insert(key.clone(), value.clone());
    }
    Value::Object(std::mem::take(left))
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating cache report directory `{}`", parent.display()))?;
    }
    let mut bytes = serde_json::to_vec_pretty(value).context("serializing cache report")?;
    bytes.push(b'\n');
    let tmp = temp_path(path);
    std::fs::write(&tmp, &bytes)
        .with_context(|| format!("writing cache report `{}`", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| {
        format!(
            "installing cache report `{}` from `{}`",
            path.display(),
            tmp.display()
        )
    })?;
    Ok(())
}

fn temp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .unwrap_or("peony-cache-report.json");
    path.with_file_name(format!(".{file_name}.tmp-{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_report_contains_stable_code_and_details() {
        let reason = PartialRelinkFallback::SectionSizeChanged {
            section: ".text".to_string(),
            previous: 4,
            current: 8,
        };
        let value = CacheOutcome::FullEmit {
            reason: FullEmitReason::PlannerFallback(&reason),
        }
        .to_json(Path::new("a.out"));

        assert_eq!(value["action"], "full_emit");
        assert_eq!(value["reason"]["code"], "section_size_changed");
        assert_eq!(value["reason"]["section"], ".text");
        assert_eq!(value["reason"]["previous"], 4);
        assert_eq!(value["reason"]["current"], 8);
    }
}
