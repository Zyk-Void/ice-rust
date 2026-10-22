use super::*;

#[test]
fn parses_plan_windows_additional_limits_and_credits() {
    let usage = parse(
        br#"{
            "plan_type":"plus",
            "rate_limit":{"allowed":true,"limit_reached":false,
              "primary_window":{"used_percent":38,"limit_window_seconds":18000,
                                "reset_after_seconds":8040,"reset_at":1},
              "secondary_window":{"used_percent":7.5,"limit_window_seconds":604800,
                                  "reset_after_seconds":300000,"reset_at":2}},
            "credits":{"has_credits":true,"unlimited":false,"balance":"12.50"},
            "additional_rate_limits":[
              {"limit_name":"GPT-6-Astra","metered_feature":"x",
               "rate_limit":{"allowed":true,"limit_reached":true,
                 "primary_window":{"used_percent":100,"limit_window_seconds":18000,
                                   "reset_after_seconds":60,"reset_at":3}}},
              {"limit_name":"No details","metered_feature":"y","rate_limit":null}]
        }"#,
    )
    .unwrap();

    assert_eq!(usage.plan, "plus");
    assert_eq!(usage.limits.len(), 2);
    assert_eq!(usage.limits[0].name, "Codex");
    assert_eq!(
        usage.limits[0].windows,
        vec![
            UsageWindow {
                used_percent: 38.0,
                window_seconds: 18_000,
                resets_in_seconds: 8_040
            },
            UsageWindow {
                used_percent: 7.5,
                window_seconds: 604_800,
                resets_in_seconds: 300_000
            },
        ]
    );
    assert!(usage.limits[1].limit_reached);
    assert_eq!(
        usage.credits,
        Some(UsageCredits {
            unlimited: false,
            balance: Some("12.50".into())
        })
    );
}

#[test]
fn tolerates_missing_sections_and_clamps_percentages() {
    let usage = parse(
        br#"{"plan_type":"free","credits":{"has_credits":false,"unlimited":false},
            "rate_limit":{"limit_reached":false,
              "primary_window":{"used_percent":140,"limit_window_seconds":-1,"reset_after_seconds":-5}}}"#,
    )
    .unwrap();
    assert_eq!(usage.credits, None);
    let window = &usage.limits[0].windows[0];
    assert_eq!(
        (
            window.used_percent,
            window.window_seconds,
            window.resets_in_seconds
        ),
        (100.0, 0, 0)
    );

    assert_eq!(parse(b"{}").unwrap().plan, "unknown");
    assert_eq!(
        parse(b"[]").unwrap_err(),
        "Unrecognized usage response (JSON is an array)"
    );
    assert_eq!(
        parse(b"<html>").unwrap_err(),
        "Unrecognized usage response (not JSON)"
    );
}

#[test]
fn explicit_nulls_numeric_strings_and_unknown_fields_do_not_fail_the_parse() {
    let usage = parse(
        br#"{"plan_type":"pro","account_id":"a","spend_control":{"reached":false},
            "rate_limit_reset_credits":{"available_count":3},
            "rate_limit":{"allowed":true,"limit_reached":null,
              "primary_window":{"used_percent":"42.5","limit_window_seconds":null,
                                "reset_after_seconds":120},
              "secondary_window":null},
            "credits":{"has_credits":true,"unlimited":false,"balance":12.5,
                       "approx_local_messages":null},
            "additional_rate_limits":null}"#,
    )
    .unwrap();
    assert_eq!(usage.plan, "pro");
    assert_eq!(
        usage.limits[0].windows,
        vec![UsageWindow {
            used_percent: 42.5,
            window_seconds: 0,
            resets_in_seconds: 120
        }]
    );
    assert_eq!(usage.credits.unwrap().balance.as_deref(), Some("12.5"));

    let empty = parse(br#"{"plan_type":null,"rate_limit":null,"credits":null}"#).unwrap();
    assert_eq!((empty.plan.as_str(), empty.limits.len()), ("unknown", 0));
}
