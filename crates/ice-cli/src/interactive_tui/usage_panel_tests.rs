use super::*;
use crate::codex_usage::{UsageCredits, UsageLimit};

fn plain(lines: &[String]) -> String {
    let mut out = String::new();
    for line in lines {
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out.push('\n');
    }
    out
}

fn panel(chatgpt: ChatGptSection) -> UsagePanel {
    UsagePanel {
        chatgpt,
        session: SessionSection {
            messages: 2,
            cached_tokens: 75,
            uncached_tokens: 25,
            cost: 0.0,
            all_history: Some(AllHistory {
                input: 1390,
                output: 53,
                cache_read: 0,
                cache_write: 0,
                total_tokens: 1443,
                cost: 0.0,
                context: Some(ContextOccupancy {
                    tokens: Some(1443),
                    window: 272_000,
                    percent: Some(0.5),
                }),
            }),
            footer: vec!["lane main · leaf abc".into()],
        },
    }
}

#[test]
fn publishing_usage_replaces_previous_panel_and_ignores_stale_results() {
    use crate::interactive_tui::publish_usage_panel;
    let chat = crate::interactive_tui::Container::new();
    let mut current = None;
    assert!(publish_usage_panel(
        &chat,
        &mut current,
        1,
        1,
        panel(ChatGptSection::SignedOut)
    ));
    let first = current.clone().unwrap();
    assert!(!publish_usage_panel(
        &chat,
        &mut current,
        2,
        1,
        panel(ChatGptSection::SignedOut)
    ));
    assert!(std::sync::Arc::ptr_eq(&first, current.as_ref().unwrap()));
    assert!(publish_usage_panel(
        &chat,
        &mut current,
        2,
        2,
        panel(ChatGptSection::SignedOut)
    ));
    assert_eq!(chat.child_count(), 1);
    assert!(!std::sync::Arc::ptr_eq(&first, current.as_ref().unwrap()));
}

fn plus_usage() -> ChatGptUsage {
    ChatGptUsage {
        plan: "plus".into(),
        limits: vec![UsageLimit {
            name: "Codex".into(),
            limit_reached: false,
            windows: vec![
                UsageWindow {
                    used_percent: 38.0,
                    window_seconds: 18_000,
                    resets_in_seconds: 8_040,
                },
                UsageWindow {
                    used_percent: 90.0,
                    window_seconds: 604_800,
                    resets_in_seconds: 300_000,
                },
            ],
        }],
        credits: Some(UsageCredits {
            unlimited: false,
            balance: Some("12.50".into()),
        }),
    }
}

#[test]
fn cache_bar_matches_the_displayed_hit_ratio() {
    let text = plain(&panel(ChatGptSection::SignedOut).session_lines());
    let line = text.lines().find(|line| line.contains("Cache")).unwrap();
    assert!(line.contains("75%"));
    assert_eq!(line.chars().filter(|c| *c == '█').count(), 18);
    assert_eq!(line.chars().filter(|c| *c == '░').count(), 6);
}

#[test]
fn bars_show_any_use_and_never_overflow() {
    assert_eq!(bar_cells(0.0, 24), (0, 24));
    assert_eq!(bar_cells(0.5, 24), (1, 23));
    assert_eq!(bar_cells(50.0, 24), (12, 12));
    assert_eq!(bar_cells(250.0, 24), (24, 0));
    assert_eq!(bar_cells(-3.0, 24), (0, 24));
}

#[test]
fn durations_labels_and_numbers_are_human_readable() {
    assert_eq!(format_duration(0), "<1m");
    assert_eq!(format_duration(59), "<1m");
    assert_eq!(format_duration(45 * 60), "45m");
    assert_eq!(format_duration(8_040), "2h 14m");
    assert_eq!(format_duration(300_000), "3d 11h");
    assert_eq!(window_label(18_000), "5h");
    assert_eq!(window_label(604_800), "weekly");
    assert_eq!(window_label(172_800), "2d");
    assert_eq!(window_label(0), "window");
    assert_eq!(plan_label("plus"), "Plus");
    assert_eq!(thousands(1_443), "1,443");
    assert_eq!(thousands(272_000), "272,000");
    assert_eq!(thousands(12), "12");
}

#[test]
fn renders_banner_plan_limits_credits_and_session_totals() {
    let text = plain(&panel(ChatGptSection::Usage(plus_usage())).render(100));
    for expected in [
        "█ █ █▀ ▄▀█ █▀▀ █▀▀",
        "ChatGPT Plus plan",
        "5h",
        "38%",
        "resets in 2h 14m",
        "weekly",
        "90%",
        "resets in 3d 11h",
        "Credits",
        "12.50 remaining",
        "This session",
        "2 messages",
        "75%",
        "All history, including abandoned branches",
        "1,443 tokens",
        "active context (estimated): 1,443 / 272,000 tokens",
        "lane main · leaf abc",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
    }
}

#[test]
fn signed_out_and_failed_states_explain_themselves() {
    let signed_out = plain(&panel(ChatGptSection::SignedOut).render(100));
    assert!(signed_out.contains("Not signed in. Use /login"));
    let failed = plain(&panel(ChatGptSection::Failed("HTTP 401".into())).render(100));
    assert!(failed.contains("Could not fetch usage: HTTP 401"));
}

#[test]
fn rendering_respects_narrow_widths() {
    for width in [0usize, 1, 20, 40] {
        for line in panel(ChatGptSection::Usage(plus_usage())).render(width) {
            assert!(
                ice_tui::visible_width(&line) <= width,
                "width {width}: {line:?}"
            );
        }
    }
}

#[test]
fn bars_line_up_in_one_column() {
    let text = plain(&panel(ChatGptSection::Usage(plus_usage())).render(100));
    let columns: Vec<usize> = text
        .lines()
        .filter(|line| {
            ["  5h", "  weekly", "  Cache", "  Context"]
                .iter()
                .any(|p| line.starts_with(p))
        })
        .map(|line| line.chars().position(|c| c == '█' || c == '░').unwrap())
        .collect();
    assert_eq!(columns.len(), 4);
    assert!(
        columns.iter().all(|column| *column == columns[0]),
        "{columns:?}"
    );
}
