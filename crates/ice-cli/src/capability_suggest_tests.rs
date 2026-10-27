use super::*;

fn descriptor(
    id: &str,
    kind: CapabilityKind,
    name: &str,
    description: &str,
    source: &str,
    available: bool,
    model_invocable: bool,
) -> CapabilityDescriptor {
    CapabilityDescriptor {
        id: id.into(),
        kind,
        name: name.into(),
        description: description.into(),
        source: source.into(),
        content_hash: "hash".into(),
        available,
        model_invocable,
    }
}

fn skill(id: &str, name: &str, description: &str) -> CapabilityDescriptor {
    descriptor(
        id,
        CapabilityKind::Skill,
        name,
        description,
        "project",
        true,
        true,
    )
}

fn tool(id: &str, server: &str, name: &str, description: &str) -> CapabilityDescriptor {
    descriptor(
        id,
        CapabilityKind::McpTool,
        name,
        description,
        &format!("mcp:{server}"),
        true,
        true,
    )
}

fn context<'a>(
    query: &'a str,
    coverage: &'a HashMap<String, SkillContextState>,
    already_suggested: &'a HashSet<String>,
    task_event: TaskEvent,
) -> SuggestContext<'a> {
    SuggestContext {
        query,
        coverage,
        already_suggested,
        task_event,
    }
}

fn suggest<'a>(
    entries: Vec<CapabilityDescriptor>,
    query: &'a str,
    coverage: &'a HashMap<String, SkillContextState>,
    already_suggested: &'a HashSet<String>,
) -> Suggestion {
    suggest_capabilities(
        &CapabilityCatalog { entries },
        &context(query, coverage, already_suggested, TaskEvent::TaskStart),
    )
}

#[test]
fn planning_skill_name_match_returns_load_hint() {
    let catalog = CapabilityCatalog {
        entries: vec![skill("skill:project:planning", "planning", "Plan work")],
    };
    let suggestion = suggest_capabilities(
        &catalog,
        &context(
            "Use the planning skill for this task",
            &HashMap::new(),
            &HashSet::new(),
            TaskEvent::TaskStart,
        ),
    );
    assert_eq!(
        suggestion,
        Suggestion::Skill {
            id: "skill:project:planning".into(),
            name: "planning".into(),
            reason: "name matches the request".into(),
            load_hint: "$planning".into(),
        }
    );
}

#[test]
fn loaded_skill_is_skipped_but_stale_skill_remains_eligible() {
    let catalog = CapabilityCatalog {
        entries: vec![
            skill("skill:project:loaded", "loaded", "planning review"),
            skill("skill:project:stale", "stale", "planning review"),
        ],
    };
    let coverage = HashMap::from([
        ("skill:project:loaded".into(), SkillContextState::Loaded),
        ("skill:project:stale".into(), SkillContextState::Stale),
    ]);
    let suggestion = suggest_capabilities(
        &catalog,
        &context(
            "planning review",
            &coverage,
            &HashSet::new(),
            TaskEvent::TaskStart,
        ),
    );
    assert!(matches!(suggestion, Suggestion::Skill { id, .. } if id == "skill:project:stale"));
}

#[test]
fn unavailable_tools_and_disabled_server_tools_are_never_suggested() {
    let catalog = CapabilityCatalog {
        entries: vec![
            descriptor(
                "mcp:offline:search",
                CapabilityKind::McpTool,
                "search",
                "find planning details",
                "mcp:offline",
                false,
                true,
            ),
            tool(
                "mcp:disabled:lookup",
                "disabled",
                "lookup",
                "find planning details",
            ),
        ],
    };
    let mut catalog = catalog;
    catalog.entries[1].available = false;
    assert_eq!(
        suggest_capabilities(
            &catalog,
            &context(
                "find planning details",
                &HashMap::new(),
                &HashSet::new(),
                TaskEvent::TaskStart,
            ),
        ),
        Suggestion::None
    );
}

#[test]
fn model_invocation_disabled_skill_is_never_suggested() {
    let catalog = CapabilityCatalog {
        entries: vec![descriptor(
            "skill:project:hidden",
            CapabilityKind::Skill,
            "hidden",
            "planning details",
            "project",
            true,
            false,
        )],
    };
    assert_eq!(
        suggest_capabilities(
            &catalog,
            &context(
                "planning details",
                &HashMap::new(),
                &HashSet::new(),
                TaskEvent::TaskStart,
            ),
        ),
        Suggestion::None
    );
}

#[test]
fn unrelated_query_returns_none() {
    assert_eq!(
        suggest(
            vec![skill("skill:project:planning", "planning", "plan work")],
            "summarize a poem",
            &HashMap::new(),
            &HashSet::new(),
        ),
        Suggestion::None
    );
}

#[test]
fn recorded_suggestion_suppresses_repeat_task_match() {
    let catalog = CapabilityCatalog {
        entries: vec![skill("skill:project:planning", "planning", "Plan work")],
    };
    let suggestion = suggest_capabilities(
        &catalog,
        &context(
            "planning",
            &HashMap::new(),
            &HashSet::new(),
            TaskEvent::TaskStart,
        ),
    );
    let mut state = SuggestState::default();
    state.record(&suggestion);
    assert_eq!(
        state.suggested(),
        &HashSet::from(["skill:project:planning".into()])
    );
    assert_eq!(
        suggest_capabilities(
            &catalog,
            &context(
                "planning",
                &HashMap::new(),
                state.suggested(),
                TaskEvent::TaskStart,
            ),
        ),
        Suggestion::None
    );
}

#[test]
fn equally_relevant_candidates_are_ambiguous_in_deterministic_order() {
    let catalog = CapabilityCatalog {
        entries: vec![
            skill("skill:project:zeta", "zeta", "handle planning"),
            skill("skill:project:alpha", "alpha", "handle planning"),
        ],
    };
    let suggestion = suggest_capabilities(
        &catalog,
        &context(
            "handle planning",
            &HashMap::new(),
            &HashSet::new(),
            TaskEvent::TaskStart,
        ),
    );
    assert_eq!(
        suggestion,
        Suggestion::Ambiguous {
            candidates: vec!["skill:project:alpha".into(), "skill:project:zeta".into()]
        }
    );
}

#[test]
fn ambiguity_is_capped_at_four_candidates() {
    let entries = (0..6)
        .map(|index| {
            skill(
                &format!("skill:project:item{index}"),
                &format!("item{index}"),
                "handle planning",
            )
        })
        .collect();
    let suggestion = suggest(entries, "handle planning", &HashMap::new(), &HashSet::new());
    assert!(matches!(suggestion, Suggestion::Ambiguous { candidates } if candidates.len() <= 4));
}

#[test]
fn inventory_question_uses_only_available_catalog_entries_and_caps_lists() {
    let mut entries: Vec<_> = (0..14)
        .map(|index| {
            skill(
                &format!("skill:project:item{index}"),
                &format!("item{index}"),
                "One line",
            )
        })
        .collect();
    entries.push(tool("mcp:server:find", "server", "find", "Search"));
    entries.push(descriptor(
        "mcp:offline:secret",
        CapabilityKind::McpTool,
        "secret",
        "Unavailable",
        "mcp:offline",
        false,
        true,
    ));
    entries.push(descriptor(
        "skill:project:hidden",
        CapabilityKind::Skill,
        "hidden",
        "Not invocable",
        "project",
        true,
        false,
    ));
    let suggestion = suggest_capabilities(
        &CapabilityCatalog { entries },
        &context(
            "WHAT SKILLS and tools can I use?",
            &HashMap::new(),
            &HashSet::new(),
            TaskEvent::TaskStart,
        ),
    );
    let Suggestion::Inventory { skills, tools } = suggestion else {
        panic!("expected catalog inventory");
    };
    assert_eq!(skills.len(), 12);
    assert_eq!(tools, vec!["find — Search"]);
    assert!(!skills.iter().any(|entry| entry.contains("hidden")));
}

#[test]
fn inventory_event_returns_inventory_even_without_inventory_phrase() {
    let suggestion = suggest_capabilities(
        &CapabilityCatalog {
            entries: vec![skill("skill:project:planning", "planning", "Plan work")],
        },
        &context(
            "planning",
            &HashMap::new(),
            &HashSet::new(),
            TaskEvent::InventoryQuestion,
        ),
    );
    assert!(matches!(suggestion, Suggestion::Inventory { .. }));
}

#[test]
fn inventory_phrase_detection_is_case_insensitive() {
    for query in [
        "what skills are installed?",
        "Which skills help?",
        "list skills",
        "available skills",
        "what tools are ready?",
        "which tools",
        "list tools",
        "available tools",
    ] {
        assert!(is_inventory_question(query), "{query}");
    }
}

#[test]
fn native_cognee_search_suppresses_mcp_memory_route() {
    let native = descriptor(
        "builtin:cognee_search",
        CapabilityKind::McpTool,
        "cognee_search",
        "Search memory",
        "builtin",
        true,
        true,
    );
    let mcp = tool(
        "mcp:memory:cognee_recall",
        "memory",
        "cognee_recall",
        "Search memory",
    );
    let preferred = prefer_native(&[&native, &mcp]);
    assert_eq!(preferred, vec![&native]);
}

#[test]
fn mcp_memory_tool_is_allowed_when_native_search_is_absent() {
    let mcp = tool(
        "mcp:memory:cognee_recall",
        "memory",
        "cognee_recall",
        "Search memory",
    );
    assert_eq!(prefer_native(&[&mcp]), vec![&mcp]);
}

#[test]
fn tool_suggestion_contains_at_most_two_handles() {
    let entries = (0..4)
        .map(|index| {
            tool(
                &format!("mcp:server:find{index}"),
                "server",
                &format!("find{index}"),
                "search planning",
            )
        })
        .collect();
    let suggestion = suggest(
        entries,
        "find0 search planning",
        &HashMap::new(),
        &HashSet::new(),
    );
    assert!(matches!(suggestion, Suggestion::Tools { handles, .. } if handles.len() <= 2));
}

#[test]
fn rendered_hints_are_bounded_and_omit_skill_bodies() {
    let body = "SECRET FULL SKILL BODY";
    let suggestion = Suggestion::Skill {
        id: "skill:project:planning".into(),
        name: "planning".into(),
        reason: "reason".into(),
        load_hint: "$planning".into(),
    };
    let rendered = render_suggestion(&suggestion).unwrap();
    assert!(rendered.encode_utf16().count() <= 280);
    assert!(!rendered.contains(body));
    assert!(rendered.starts_with("Capability hint (advisory):"));
    assert!(render_suggestion(&Suggestion::None).is_none());
}

#[test]
fn inventory_descriptions_truncate_at_utf16_boundary() {
    let description = format!("{}🦀tail", "a".repeat(78));
    let suggestion = suggest_capabilities(
        &CapabilityCatalog {
            entries: vec![skill("skill:project:long", "long", &description)],
        },
        &context(
            "what skills are available?",
            &HashMap::new(),
            &HashSet::new(),
            TaskEvent::TaskStart,
        ),
    );
    let Suggestion::Inventory { skills, .. } = suggestion else {
        panic!("expected inventory");
    };
    let rendered = render_suggestion(&Suggestion::Inventory {
        skills,
        tools: vec![],
    })
    .unwrap();
    assert!(rendered.encode_utf16().count() <= 280);
    assert!(rendered.contains(&format!("{}…", "a".repeat(78))));
    assert!(!rendered.contains("🦀"));
}

#[test]
fn inventory_and_tool_rendering_have_advisory_prefix() {
    let inventory = render_suggestion(&Suggestion::Inventory {
        skills: vec!["planning — Plan work".into()],
        tools: vec!["server/find — Search".into()],
    })
    .unwrap();
    assert!(inventory.starts_with("Capability inventory (advisory):"));

    let tools = render_suggestion(&Suggestion::Tools {
        handles: vec![ToolHandle {
            id: "mcp:server:find".into(),
            server: "server".into(),
            name: "find".into(),
        }],
        reason: "only relevant capability".into(),
    })
    .unwrap();
    assert!(tools.starts_with("Capability hint (advisory):"));
}
