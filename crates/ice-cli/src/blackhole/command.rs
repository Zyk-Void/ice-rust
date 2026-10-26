//! `/blackhole` and the Blackhole settings section. Ports `index.ts`
//! (BLACK-005, BLACK-006).

use std::path::Path;

use super::config::{
    config_path, load_config_with, percent_label, save_config_with, CompactionEngine,
    CompactionMode, ConfigPatch, EnvLookup, MidRunCompaction, TailBehavior,
};
use super::js::js_number;
use crate::builtin_extensions::{NoticeLevel, SettingRow};

const USAGE: &str = "Usage: /blackhole [status|percent <1-99>|resume|pause|off]";

/// Run `/blackhole <args>` and return the notice to show.
pub fn run_command(agent_dir: &Path, env: EnvLookup<'_>, args: &str) -> (NoticeLevel, String) {
    let mut parts = args.split_whitespace();
    let command = parts.next();
    let value = parts.next();
    let save = |patch: ConfigPatch| save_config_with(agent_dir, env, &patch);
    match command {
        None | Some("status") => {
            let config = load_config_with(agent_dir, env, &mut |_| {});
            let refine = if config.refine {
                match &config.refine_model {
                    Some(model) => format!("on ({model})"),
                    None => "on".into(),
                }
            } else {
                "off".into()
            };
            (
                NoticeLevel::Info,
                format!(
                    "Blackhole: {}, {}%, tail {}, trim kept output {}, refine {refine}; config {}",
                    config.mid_run_compaction.as_str(),
                    percent_label(&config),
                    config.tail_behavior.as_str(),
                    if config.trim_kept_tool_results {
                        "on"
                    } else {
                        "off"
                    },
                    config_path(agent_dir).display()
                ),
            )
        }
        Some("percent") => {
            let threshold = js_number(value.unwrap_or("undefined"));
            if !(threshold.is_finite() && (1.0..=99.0).contains(&threshold)) {
                return (NoticeLevel::Warning, USAGE.into());
            }
            match save(ConfigPatch {
                compaction: Some(CompactionMode::Auto),
                compaction_engine: Some(CompactionEngine::Blackhole),
                compact_after_percent: Some(threshold),
                ..ConfigPatch::default()
            }) {
                Ok(config) => (
                    NoticeLevel::Info,
                    format!("Blackhole threshold: {}%", percent_label(&config)),
                ),
                Err(error) => (
                    NoticeLevel::Error,
                    format!("Could not save Blackhole config: {error}"),
                ),
            }
        }
        Some(mode @ ("resume" | "pause" | "off")) => {
            let mid_run = MidRunCompaction::parse(mode).expect("matched mode");
            match save(ConfigPatch {
                compaction: Some(if mode == "off" {
                    CompactionMode::Off
                } else {
                    CompactionMode::Auto
                }),
                compaction_engine: Some(CompactionEngine::Blackhole),
                mid_run_compaction: Some(mid_run),
                ..ConfigPatch::default()
            }) {
                Ok(config) => (
                    NoticeLevel::Info,
                    format!(
                        "Blackhole mid-run compaction: {}",
                        config.mid_run_compaction.as_str()
                    ),
                ),
                Err(error) => (
                    NoticeLevel::Error,
                    format!("Could not save Blackhole config: {error}"),
                ),
            }
        }
        Some(_) => (NoticeLevel::Warning, USAGE.into()),
    }
}

fn on_off(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

/// The eight Blackhole rows, read fresh from the effective config.
pub fn settings_rows(agent_dir: &Path, env: EnvLookup<'_>) -> Vec<SettingRow> {
    let config = load_config_with(agent_dir, env, &mut |_| {});
    let percent_values: Vec<String> = (1..=99).map(|percent| format!("{percent}%")).collect();
    vec![
        SettingRow::new(
            "blackhole.compaction",
            "Blackhole compaction",
            "Blackhole compaction policy",
            config.compaction.as_str(),
            &["auto", "manual", "off"],
        ),
        SettingRow::new(
            "blackhole.compaction-engine",
            "Blackhole engine",
            "Summary engine used when Blackhole compaction runs",
            config.compaction_engine.as_str(),
            &["blackhole", "ice-default"],
        ),
        SettingRow::new(
            "blackhole.mid-run-compaction",
            "Blackhole mid-run",
            "Compact after a tool turn and pause or resume the task",
            config.mid_run_compaction.as_str(),
            &["off", "pause", "resume"],
        ),
        SettingRow {
            key: "blackhole.threshold".into(),
            label: "Blackhole compaction threshold".into(),
            description: "Percentage of the active model context window; resolves per selected model"
                .into(),
            current: format!("{}%", percent_label(&config)),
            values: percent_values,
        },
        SettingRow::new(
            "blackhole.tail-behavior",
            "Blackhole tail",
            "Tail preservation strategy used by the summary",
            config.tail_behavior.as_str(),
            &["ice-default", "minimal"],
        ),
        SettingRow::new(
            "blackhole.trim-kept-tool-results",
            "Blackhole trim kept tool output",
            "Cap large tool results kept from before compaction in model context; the session keeps full output",
            on_off(config.trim_kept_tool_results),
            &["true", "false"],
        ),
        SettingRow::new(
            "blackhole.refine",
            "Blackhole refine",
            "Add a short model-written state-of-work section to each summary (one extra model call)",
            on_off(config.refine),
            &["false", "true"],
        ),
        SettingRow::new(
            "blackhole.memory",
            "Blackhole memory",
            "Persist the memory setting; memory workers are not currently implemented",
            on_off(config.memory),
            &["false", "true"],
        ),
    ]
}

/// Apply one settings row change. `None` when the key is not a Blackhole row.
pub fn apply_setting(
    agent_dir: &Path,
    env: EnvLookup<'_>,
    key: &str,
    value: &str,
) -> Option<Result<String, String>> {
    let id = key.strip_prefix("blackhole.")?;
    let patch = match id {
        "compaction" => ConfigPatch {
            compaction: Some(
                match parse_setting_choice(id, value, CompactionMode::parse) {
                    Ok(value) => value,
                    Err(error) => return Some(Err(error)),
                },
            ),
            ..ConfigPatch::default()
        },
        "compaction-engine" => ConfigPatch {
            compaction_engine: Some(
                match parse_setting_choice(id, value, CompactionEngine::parse) {
                    Ok(value) => value,
                    Err(error) => return Some(Err(error)),
                },
            ),
            ..ConfigPatch::default()
        },
        "mid-run-compaction" => ConfigPatch {
            mid_run_compaction: Some(
                match parse_setting_choice(id, value, MidRunCompaction::parse) {
                    Ok(value) => value,
                    Err(error) => return Some(Err(error)),
                },
            ),
            ..ConfigPatch::default()
        },
        "threshold" => {
            let text = value.trim();
            let number = text.strip_suffix('%').unwrap_or(text);
            let Some(threshold) = number
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite() && (1.0..=99.0).contains(value))
            else {
                return Some(Err(
                    "Blackhole threshold must be a number from 1 to 99".into()
                ));
            };
            ConfigPatch {
                compact_after_percent: Some(threshold),
                ..ConfigPatch::default()
            }
        }
        "tail-behavior" => ConfigPatch {
            tail_behavior: Some(match parse_setting_choice(id, value, TailBehavior::parse) {
                Ok(value) => value,
                Err(error) => return Some(Err(error)),
            }),
            ..ConfigPatch::default()
        },
        "trim-kept-tool-results" => ConfigPatch {
            trim_kept_tool_results: Some(match parse_setting_boolean(id, value) {
                Ok(value) => value,
                Err(error) => return Some(Err(error)),
            }),
            ..ConfigPatch::default()
        },
        "refine" => ConfigPatch {
            refine: Some(match parse_setting_boolean(id, value) {
                Ok(value) => value,
                Err(error) => return Some(Err(error)),
            }),
            ..ConfigPatch::default()
        },
        "memory" => ConfigPatch {
            memory: Some(match parse_setting_boolean(id, value) {
                Ok(value) => value,
                Err(error) => return Some(Err(error)),
            }),
            ..ConfigPatch::default()
        },
        _ => return Some(Err(format!("Unknown Blackhole setting {id}"))),
    };
    Some(
        save_config_with(agent_dir, env, &patch)
            .map(|_| format!("Blackhole {id}: {value}"))
            .map_err(|error| format!("Could not save Blackhole config: {error}")),
    )
}

fn parse_setting_choice<T>(
    id: &str,
    value: &str,
    parse: fn(&str) -> Option<T>,
) -> Result<T, String> {
    parse(value).ok_or_else(|| format!("Invalid value for Blackhole setting {id}"))
}

fn parse_setting_boolean(id: &str, value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("Invalid value for Blackhole setting {id}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn status_reports_defaults_and_path() {
        let dir = tempfile::tempdir().unwrap();
        let (level, text) = run_command(dir.path(), &no_env, "");
        assert_eq!(level, NoticeLevel::Info);
        assert!(text.starts_with(
            "Blackhole: off, 85%, tail minimal, trim kept output on, refine off; config "
        ));
    }

    #[test]
    fn percent_accepts_fractions_and_rejects_out_of_range() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            run_command(dir.path(), &no_env, "percent 12.5 extra"),
            (NoticeLevel::Info, "Blackhole threshold: 12.5%".into())
        );
        assert_eq!(
            run_command(dir.path(), &no_env, "percent 100").0,
            NoticeLevel::Warning
        );
        assert_eq!(
            run_command(dir.path(), &no_env, "percent").0,
            NoticeLevel::Warning
        );
        assert_eq!(
            run_command(dir.path(), &no_env, "nope").0,
            NoticeLevel::Warning
        );
    }

    #[test]
    fn mode_presets_set_engine_and_compaction() {
        let dir = tempfile::tempdir().unwrap();
        run_command(dir.path(), &no_env, "off");
        let config = load_config_with(dir.path(), &no_env, &mut |_| {});
        assert_eq!(config.compaction, CompactionMode::Off);
        assert_eq!(config.mid_run_compaction, MidRunCompaction::Off);
        run_command(dir.path(), &no_env, "pause");
        let config = load_config_with(dir.path(), &no_env, &mut |_| {});
        assert_eq!(config.compaction, CompactionMode::Auto);
        assert_eq!(config.mid_run_compaction, MidRunCompaction::Pause);
    }

    #[test]
    fn settings_rows_and_changes_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let rows = settings_rows(dir.path(), &no_env);
        assert_eq!(rows.len(), 8);
        assert_eq!(rows[3].current, "85%");
        assert_eq!(rows[3].values.len(), 99);
        assert!(
            apply_setting(dir.path(), &no_env, "blackhole.threshold", "40%")
                .unwrap()
                .is_ok()
        );
        assert!(
            apply_setting(dir.path(), &no_env, "blackhole.threshold", "12.5%")
                .unwrap()
                .is_ok()
        );
        assert!(
            apply_setting(dir.path(), &no_env, "blackhole.threshold", "")
                .unwrap()
                .is_err()
        );
        assert!(
            apply_setting(dir.path(), &no_env, "blackhole.threshold", "not a percent")
                .unwrap()
                .is_err()
        );
        assert!(
            apply_setting(dir.path(), &no_env, "blackhole.refine", "true")
                .unwrap()
                .is_ok()
        );
        let before_invalid_updates = load_config_with(dir.path(), &no_env, &mut |_| {});
        for key in [
            "blackhole.trim-kept-tool-results",
            "blackhole.refine",
            "blackhole.memory",
            "blackhole.compaction",
            "blackhole.compaction-engine",
            "blackhole.mid-run-compaction",
            "blackhole.tail-behavior",
        ] {
            assert!(apply_setting(dir.path(), &no_env, key, "tru")
                .unwrap()
                .is_err());
        }
        let config = load_config_with(dir.path(), &no_env, &mut |_| {});
        assert_eq!(config.compact_after_percent, 12.5);
        assert!(config.refine);
        assert_eq!(
            config.trim_kept_tool_results,
            before_invalid_updates.trim_kept_tool_results
        );
        assert_eq!(config.refine, before_invalid_updates.refine);
        assert_eq!(config.memory, before_invalid_updates.memory);
        assert!(apply_setting(dir.path(), &no_env, "theme", "dark").is_none());
    }
}
