//! Local, inert capability metadata. No skill bodies or MCP connections are used.
//!
//! Existing `skillDirs` settings can explicitly include `~/.agents/skills`;
//! the catalog does not discover that directory automatically.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use ice_harness::diagnostics::{ResourceDiagnostic, ResourceType};
use ice_harness::skills::LoadSkillsResult;
use ice_harness::types::Skill;
use sha2::{Digest, Sha256};

use crate::mcp::{ServerStatus, ServerView};
use crate::skill_coverage::SkillContextState;

/// Roots already supplied to the skill loader. More specific paths win; ties
/// prefer explicit settings, then builtin, package, project and global.
#[derive(Clone, Debug, Default)]
pub struct SkillSourceDirs {
    pub project: Vec<PathBuf>,
    pub global: Vec<PathBuf>,
    pub package: Vec<PathBuf>,
    pub builtin: Vec<PathBuf>,
    pub settings: Vec<PathBuf>,
}

impl SkillSourceDirs {
    pub fn classify(&self, path: &Path) -> &'static str {
        let mut best = (0, "other");
        for (source, roots) in [
            ("settings", &self.settings),
            ("builtin", &self.builtin),
            ("package", &self.package),
            ("project", &self.project),
            ("global", &self.global),
        ] {
            for root in roots {
                let depth = root.components().count();
                if depth > best.0 && path.starts_with(root) {
                    best = (depth, source);
                }
            }
        }
        best.1
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapabilityKind {
    Skill,
    McpTool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityDescriptor {
    pub id: String,
    pub kind: CapabilityKind,
    pub name: String,
    pub description: String,
    pub source: String,
    pub content_hash: String,
    pub available: bool,
    pub model_invocable: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapabilityCatalog {
    pub entries: Vec<CapabilityDescriptor>,
}

impl CapabilityCatalog {
    /// Skills must be supplied in loader precedence order, before name deduplication.
    /// Each source label identifies the registry that supplied the skill.
    pub fn build(skills: &[(&str, Skill)], servers: &[ServerView]) -> Self {
        let mut entries = Vec::new();
        let mut seen = HashSet::new();
        let mut counts = HashMap::new();
        let mut ids = HashSet::new();
        for (source, skill) in skills {
            *counts
                .entry((*source, skill.name.as_str()))
                .or_insert(0usize) += 1;
        }
        for (source, skill) in skills {
            let available = seen.insert(skill.name.clone());
            let mut id = format!("skill:{source}:{}", skill.name);
            if counts[&(*source, skill.name.as_str())] > 1 {
                id.push(':');
                id.push_str(&format!("{:x}", Sha256::digest(skill.file_path.as_bytes()))[..12]);
            }
            let base_id = id.clone();
            let mut suffix = 2;
            while !ids.insert(id.clone()) {
                id = format!("{base_id}:{suffix}");
                suffix += 1;
            }
            entries.push(descriptor(
                id,
                CapabilityKind::Skill,
                &skill.name,
                &skill.description,
                &skill.file_path,
                available,
                !skill.disable_model_invocation.unwrap_or(false),
            ));
            if let Some(entry) = entries.last_mut() {
                entry.content_hash = format!("{:x}", Sha256::digest(skill.content.as_bytes()));
            }
        }
        for server in servers {
            let source = format!("mcp:{}", server.name);
            let available =
                !server.disabled && matches!(server.status, ServerStatus::Connected { .. });
            for tool in server.catalog.iter().flatten() {
                entries.push(descriptor(
                    format!("mcp:{}:{}", server.name, tool.name),
                    CapabilityKind::McpTool,
                    &tool.name,
                    &tool.description,
                    &source,
                    available,
                    true,
                ));
                if let Some(entry) = entries.last_mut() {
                    let mut hash = Sha256::new();
                    hash.update(tool.name.as_bytes());
                    hash.update(tool.description.as_bytes());
                    entry.content_hash = format!("{:x}", hash.finalize());
                }
            }
        }
        Self { entries }
    }

    /// Snapshot the loader's winners and diagnostic-identified losers, without
    /// connecting MCP servers or reading any additional directories.
    pub fn from_live(
        result: &LoadSkillsResult,
        dirs: &SkillSourceDirs,
        servers: &[ServerView],
    ) -> Self {
        let losers: HashSet<_> = result
            .resource_diagnostics
            .iter()
            .filter_map(|diagnostic| match diagnostic {
                ResourceDiagnostic::Collision(collision)
                    if collision.resource_type == ResourceType::Skill =>
                {
                    Some(collision.loser_path.as_str())
                }
                _ => None,
            })
            .collect();
        let winner_paths: HashSet<_> = result
            .skills
            .iter()
            .map(|skill| skill.file_path.as_str())
            .collect();
        let mut paths = winner_paths.clone();
        let shadowed = result.shadowed_skills.iter().filter(|skill| {
            losers.contains(skill.file_path.as_str()) && paths.insert(skill.file_path.as_str())
        });
        let skills: Vec<_> = result
            .skills
            .iter()
            .chain(shadowed)
            .map(|skill| (dirs.classify(Path::new(&skill.file_path)), skill.clone()))
            .collect();
        let mut catalog = Self::build(&skills, servers);
        for (entry, (_, skill)) in catalog.entries.iter_mut().zip(&skills) {
            entry.available = winner_paths.contains(skill.file_path.as_str());
        }
        catalog
    }

    /// Rebuild after the caller refreshes the independent registries.
    pub fn refresh_live(
        &mut self,
        result: &LoadSkillsResult,
        dirs: &SkillSourceDirs,
        servers: &[ServerView],
    ) {
        *self = Self::from_live(result, dirs, servers);
    }

    /// Replace the snapshot after the owning registries have refreshed.
    pub fn refresh(&mut self, skills: &[(&str, Skill)], servers: &[ServerView]) {
        *self = Self::build(skills, servers);
    }

    /// Return skill descriptors with their branch-derived context state.
    /// MCP tools do not have full instruction bodies and are omitted.
    pub fn with_context_state(
        &self,
        coverage: &HashMap<String, SkillContextState>,
    ) -> Vec<(&CapabilityDescriptor, SkillContextState)> {
        self.entries
            .iter()
            .filter(|entry| entry.kind == CapabilityKind::Skill)
            .map(|entry| {
                (
                    entry,
                    coverage
                        .get(&entry.id)
                        .copied()
                        .unwrap_or(SkillContextState::Advertised),
                )
            })
            .collect()
    }

    pub fn shortlist(&self, query: &str, limit: usize) -> Vec<&CapabilityDescriptor> {
        let query = query.trim().to_lowercase();
        let tokens: HashSet<&str> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .collect();
        if tokens.is_empty() {
            return Vec::new();
        }
        let mut matches: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| entry.available && entry.model_invocable)
            .filter_map(|entry| {
                let text = format!("{} {}", entry.name, entry.description).to_lowercase();
                let words: HashSet<&str> = text
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|s| !s.is_empty())
                    .collect();
                let score = tokens.intersection(&words).count();
                (score > 0).then_some((entry.name.to_lowercase() == query, score, entry))
            })
            .collect();
        matches.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| a.2.id.cmp(&b.2.id))
        });
        matches
            .into_iter()
            .take(limit.min(8))
            .map(|(_, _, entry)| entry)
            .collect()
    }
}

fn descriptor(
    id: String,
    kind: CapabilityKind,
    name: &str,
    description: &str,
    source: &str,
    available: bool,
    model_invocable: bool,
) -> CapabilityDescriptor {
    let description: String = description.chars().take(240).collect();
    let mut hash = Sha256::new();
    for field in [name, description.as_str(), source] {
        hash.update((field.len() as u64).to_be_bytes());
        hash.update(field.as_bytes());
    }
    CapabilityDescriptor {
        id,
        kind,
        name: name.into(),
        description,
        source: source.into(),
        content_hash: format!("{:x}", hash.finalize()),
        available,
        model_invocable,
    }
}

#[cfg(test)]
#[path = "capability_catalog_tests.rs"]
mod live_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::ToolInfo;
    use std::path::PathBuf;

    fn skill(name: &str, description: &str, disabled: bool) -> Skill {
        Skill {
            name: name.into(),
            description: description.into(),
            content: "body".into(),
            file_path: String::new(),
            disable_model_invocation: Some(disabled),
        }
    }

    #[test]
    fn shadowing_visibility_and_hashes() {
        let skills = vec![
            ("project", skill("same", &"é".repeat(250), false)),
            ("global", skill("same", "other", false)),
            ("builtin", skill("hidden", "same", true)),
        ];
        let catalog = CapabilityCatalog::build(&skills, &[]);
        assert_eq!(catalog, CapabilityCatalog::build(&skills, &[]));
        assert_ne!(catalog.entries[0].id, catalog.entries[1].id);
        assert!(!catalog.entries[1].available);
        assert_eq!(catalog.entries[0].description.chars().count(), 240);
        assert_eq!(catalog.shortlist("same", 8).len(), 1);
        assert_eq!(catalog.entries[0].content_hash.len(), 64);
    }

    #[test]
    fn disconnected_tools_and_ranking() {
        let server = ServerView {
            name: "offline".into(),
            source: PathBuf::new(),
            status: ServerStatus::Failed("offline".into()),
            disabled: false,
            catalog: Some(vec![ToolInfo {
                name: "find".into(),
                description: "search".into(),
                tokens: 0,
                direct: false,
            }]),
        };
        let mut skills: Vec<_> = (0..10)
            .map(|i| ("project", skill(&format!("item{i}"), "search", false)))
            .collect();
        skills.push(("global", skill("search", "", false)));
        let mut catalog = CapabilityCatalog::build(&skills, &[server]);
        assert!(!catalog.entries.last().unwrap().available);
        let results = catalog.shortlist("search", 100);
        assert_eq!(results.len(), 8);
        assert_eq!(results[0].name, "search");
        assert!(results[1..].windows(2).all(|p| p[0].id < p[1].id));
        catalog.refresh(&[], &[]);
        assert!(catalog.entries.is_empty());
    }
}
