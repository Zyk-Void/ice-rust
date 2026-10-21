//! Mirrors `packages/agent/src/harness/session/state.ts` — the in-memory
//! mutation reducer that enforces the recoverable-session invariants.
//!
//! Critical invariants enforced here (plan §5):
//! - **Consecutive seq**: `seq == state.sequence + 1` or `invalid_entry`.
//!   Recovery from a torn tail relies on a global monotonic sequence.
//! - **Write-once ids**: entry/record ids are unique forever (`already_exists`
//!   on reuse).
//! - **Lane-leaf chaining**: an entry with `lane: Some(l)` must have
//!   `entry.parent_id == lanes[l]` (the lane's current leaf). Otherwise the
//!   branch would fork silently.
//! - **Fork-without-lane**: the fork path passes `lane: None`, so chaining is
//!   *not* checked — the forked entries arrive with their original `parent_id`
//!   chain intact, and a subsequent `Lane { lane, leaf_id }` mutation re-points
//!   the lane.
//! - **At-most-one open op per lane**: a 2nd `operation_started` while one is
//!   open is *not* rejected here (state allows the map to hold multiple, to
//!   support the "aborting while suspended" snapshot path); the **reducer**
//!   (`reducer.rs`) flags `multiple_open_operations` as corruption, and
//!   recovery's `find_open_operations(limit: 2)` interprets `>=2` as corruption.
//!
//! Note: the TS `applyMutation` allows multiple open operations to coexist in
//! `openOperationsByLane` (it only deletes on `operation_finished`). The
//! "at-most-one" rule is a *reducer-level* corruption check plus a harness-level
//! precondition, not a state-level hard reject. This port mirrors that exactly.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::error::{SessionError, SessionResult};
use crate::session::types::{
    BranchBounds, Entry, EntryOrder, EntryQuery, ForkOptions, ForkPosition, LanePointer,
    LaneRecord, LogItem, LogOptions, OperationStartedRecord, RecordQuery, SessionMetadata,
    SessionMutation, SessionStats,
};

/// The in-memory mutation reducer. Mirrors TS `SessionState`. Owns the
/// append-only entry/record logs, the lane→leaf map, open operations per lane,
/// global name/labels facts, and running stats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    sequence: u64,
    used_ids: std::collections::HashSet<String>,
    entries: Vec<Entry>,
    entries_by_id: HashMap<String, Entry>,
    records: Vec<LaneRecord>,
    open_operations_by_lane: HashMap<String, BTreeMap<u64, OperationStartedRecord>>,
    lanes: BTreeMap<String, Option<String>>,
    /// Insertion order for lane projections; absent in legacy serialized states.
    #[serde(default)]
    lane_order: Vec<String>,
    log: Vec<LogItem>,
    stats: SessionStats,
    name: Option<String>,
    labels: HashMap<String, String>,
    /// Insertion order for labels, including delete/re-add semantics.
    #[serde(default)]
    labels_order: Vec<String>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionState {
    /// New state with a single `main` lane pointing at no leaf (mirrors TS
    /// `lanes = new Map([["main", null]])`).
    pub fn new() -> Self {
        let mut lanes = BTreeMap::new();
        lanes.insert("main".to_string(), None);
        Self {
            sequence: 0,
            used_ids: std::collections::HashSet::new(),
            entries: Vec::new(),
            entries_by_id: HashMap::new(),
            records: Vec::new(),
            open_operations_by_lane: HashMap::new(),
            lanes,
            lane_order: vec!["main".to_string()],
            log: Vec::new(),
            stats: SessionStats::default(),
            name: None,
            labels: HashMap::new(),
            labels_order: Vec::new(),
        }
    }

    pub fn next_sequence(&self) -> u64 {
        self.sequence + 1
    }

    /// The highest applied mutation seq (0 before any mutation). The JSONL
    /// loader uses this to detect a duplicated/late append, where a mutation
    /// arrives with an already-consumed seq.
    pub(crate) fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Advance the seq counter to `seq` (no-op when already at or past it).
    /// The JSONL loader calls this when a divergent writer's counter leaves a
    /// gap, so the next mutation is accepted instead of refused.
    pub(crate) fn resync_sequence(&mut self, seq: u64) {
        if seq > self.sequence {
            self.sequence = seq;
        }
    }

    pub fn get_lanes(&self) -> Vec<LanePointer> {
        Self::ordered_keys(&self.lane_order, self.lanes.keys().cloned())
            .into_iter()
            .filter_map(|lane| {
                self.lanes.get(&lane).map(|leaf_id| LanePointer {
                    lane,
                    leaf_id: leaf_id.clone(),
                })
            })
            .collect()
    }

    /// Return insertion order where available, with sorted keys as a stable
    /// fallback for old snapshots that predate the explicit order vectors.
    fn ordered_keys(order: &[String], keys: impl IntoIterator<Item = String>) -> Vec<String> {
        let keys: Vec<String> = keys.into_iter().collect();
        let known: HashSet<String> = keys.iter().cloned().collect();
        let mut seen = HashSet::new();
        let mut result = Vec::with_capacity(keys.len());
        for key in order {
            if known.contains(key) && seen.insert(key.clone()) {
                result.push(key.clone());
            }
        }
        let mut fallback: Vec<String> = keys
            .into_iter()
            .filter(|key| seen.insert(key.clone()))
            .collect();
        fallback.sort_unstable();
        result.extend(fallback);
        result
    }

    fn normalize_lane_order(&mut self) {
        self.lane_order = Self::ordered_keys(&self.lane_order, self.lanes.keys().cloned());
    }

    fn normalize_labels_order(&mut self) {
        self.labels_order = Self::ordered_keys(&self.labels_order, self.labels.keys().cloned());
    }

    /// Look up a lane's leaf, or `invalid_lane` if the lane doesn't exist.
    pub fn require_lane(&self, lane: &str) -> SessionResult<Option<String>> {
        self.lanes
            .get(lane)
            .cloned()
            .ok_or_else(|| SessionError::invalid_lane(format!("Lane not found: {lane}")))
    }

    /// `already_exists` if the lane already exists. Mirrors `validateNewLane`.
    pub fn validate_new_lane(&self, lane: &str) -> SessionResult<()> {
        if self.lanes.contains_key(lane) {
            return Err(SessionError::already_exists(format!(
                "Lane already exists: {lane}"
            )));
        }
        Ok(())
    }

    /// `not_found` if `target_id` is set and unknown. Mirrors `validateTarget`.
    pub fn validate_target(&self, target_id: Option<&str>) -> SessionResult<()> {
        if let Some(id) = target_id {
            if !self.entries_by_id.contains_key(id) {
                return Err(SessionError::not_found(format!("Entry not found: {id}")));
            }
        }
        Ok(())
    }

    /// `already_exists` if the id was already used. Mirrors `validateUnusedId`.
    pub fn validate_unused_id(&self, id: &str) -> SessionResult<()> {
        if self.used_ids.contains(id) {
            return Err(SessionError::already_exists(format!(
                "Session id already exists: {id}"
            )));
        }
        Ok(())
    }

    /// Apply a mutation, enforcing every invariant. Mirrors TS `applyMutation`.
    /// The `Entry` arm carries a full [`Entry`] whose `seq`/`parent_id`/`timestamp`
    /// are already stamped by the storage layer (TS shape); the state *validates*
    /// lane-leaf chaining against `entry.parent_id`, it does not set it.
    pub fn apply_mutation(&mut self, mutation: SessionMutation) -> SessionResult<ApplyOutcome> {
        let seq = mutation.seq();
        if seq != self.sequence + 1 {
            return Err(SessionError::invalid_entry(format!(
                "Invalid session mutation: has non-consecutive seq {seq}"
            )));
        }

        match mutation {
            SessionMutation::Entry {
                seq, lane, entry, ..
            } => {
                self.validate_unused_id(entry.id())?;
                // Lane chaining — only when a lane is named (fork passes None).
                if let Some(lane_name) = lane.as_deref() {
                    let leaf = self.lanes.get(lane_name).cloned().ok_or_else(|| {
                        SessionError::invalid_entry(format!(
                            "Invalid session mutation: references missing lane {lane_name}"
                        ))
                    })?;
                    // `entry.parent_id` must equal the lane's current leaf.
                    if entry.parent_id().map(|s| s.to_string()) != leaf {
                        return Err(SessionError::invalid_entry(
                            "Invalid session mutation: does not chain to the lane leaf".to_string(),
                        ));
                    }
                }
                // Parent existence (when set).
                if let Some(parent) = entry.parent_id() {
                    if !self.entries_by_id.contains_key(parent) {
                        return Err(SessionError::invalid_entry(format!(
                            "Invalid session mutation: references missing parent {parent}"
                        )));
                    }
                }

                let is_message = matches!(entry, Entry::Message(_));
                self.sequence = seq;
                self.used_ids.insert(entry.id().to_string());
                self.entries.push(entry.clone());
                self.entries_by_id
                    .insert(entry.id().to_string(), entry.clone());
                if let Some(lane_name) = lane.as_deref() {
                    if let Some(slot) = self.lanes.get_mut(lane_name) {
                        *slot = Some(entry.id().to_string());
                    }
                }
                self.log.push(LogItem::Entry {
                    seq,
                    entry: entry.clone(),
                });
                // Entries without a lane are imported history (not newly
                // authored activity), notably on fork replay. Keep their
                // structural data without charging session accounting.
                if is_message && lane.is_some() {
                    self.stats.message_count += 1;
                }
                Ok(ApplyOutcome::Entry(entry))
            }
            SessionMutation::Record { record } => {
                let lane = record.lane().to_string();
                if !self.lanes.contains_key(&lane) {
                    return Err(SessionError::invalid_entry(format!(
                        "Invalid session mutation: references missing lane {lane}"
                    )));
                }
                self.validate_unused_id(record.id())?;
                let seq = record.seq();
                self.sequence = seq;
                self.used_ids.insert(record.id().to_string());
                match &record {
                    LaneRecord::OperationStarted(started) => {
                        self.open_operations_by_lane
                            .entry(lane.clone())
                            .or_default()
                            .insert(seq, started.clone());
                    }
                    LaneRecord::OperationFinished(finished) => {
                        if let Some(open) = self.open_operations_by_lane.get_mut(&lane) {
                            // Remove by run_id (matching the operation_started id).
                            open.retain(|_, started| started.base.id != finished.run_id);
                            if open.is_empty() {
                                self.open_operations_by_lane.remove(&lane);
                            }
                        }
                    }
                    _ => {}
                }
                if let LaneRecord::Usage(usage_rec) = &record {
                    self.stats.cached_tokens += usage_rec.usage.cache_read;
                    self.stats.uncached_tokens +=
                        usage_rec.usage.input + usage_rec.usage.cache_write;
                    self.stats.total_tokens += usage_rec.usage.total_tokens;
                    self.stats.cost_total += usage_rec.usage.cost.total;
                }
                self.records.push(record.clone());
                self.log.push(LogItem::Record {
                    seq,
                    record: record.clone(),
                });
                Ok(ApplyOutcome::Record(record))
            }
            SessionMutation::Lane { seq, lane, leaf_id } => {
                if let Some(leaf) = leaf_id.as_deref() {
                    if !self.entries_by_id.contains_key(leaf) {
                        return Err(SessionError::invalid_entry(format!(
                            "Invalid session mutation: references missing lane target {leaf}"
                        )));
                    }
                }
                self.normalize_lane_order();
                let is_new_lane = !self.lanes.contains_key(&lane);
                self.sequence = seq;
                self.lanes.insert(lane.clone(), leaf_id.clone());
                if is_new_lane {
                    self.lane_order.push(lane.clone());
                }
                self.log.push(LogItem::Lane { seq, lane, leaf_id });
                Ok(ApplyOutcome::Lane)
            }
            SessionMutation::FactName { seq, name } => {
                self.sequence = seq;
                self.name = name.clone();
                self.log.push(LogItem::FactName { seq, name });
                Ok(ApplyOutcome::Fact)
            }
            SessionMutation::FactLabel {
                seq,
                target_id,
                label,
            } => {
                if !self.entries_by_id.contains_key(&target_id) {
                    return Err(SessionError::invalid_entry(format!(
                        "Invalid session mutation: references missing label target {target_id}"
                    )));
                }
                self.normalize_labels_order();
                self.sequence = seq;
                match &label {
                    Some(l) => {
                        if !self.labels.contains_key(&target_id) {
                            self.labels_order.push(target_id.clone());
                        }
                        self.labels.insert(target_id.clone(), l.clone());
                    }
                    None => {
                        self.labels.remove(&target_id);
                        self.labels_order.retain(|id| id != &target_id);
                    }
                }
                self.log.push(LogItem::FactLabel {
                    seq,
                    target_id,
                    label,
                });
                Ok(ApplyOutcome::Fact)
            }
        }
    }

    pub fn get_entry(&self, id: &str) -> Option<&Entry> {
        self.entries_by_id.get(id)
    }

    pub fn find_entries(&self, query: &EntryQuery) -> SessionResult<Vec<Entry>> {
        validate_limit(query.limit)?;
        let order = query.order.unwrap_or_default();
        let mut results = Vec::new();
        let iter: Box<dyn Iterator<Item = &Entry>> = match order {
            EntryOrder::OldestFirst => Box::new(self.entries.iter()),
            EntryOrder::NewestFirst => Box::new(self.entries.iter().rev()),
        };
        for entry in iter {
            if !self.matches_entry_query(entry, query) {
                continue;
            }
            results.push(entry.clone());
            if let Some(limit) = query.limit {
                if results.len() >= limit {
                    break;
                }
            }
        }
        Ok(results)
    }

    pub fn find_entries_on_branch(
        &self,
        query: &EntryQuery,
        bounds: &BranchBounds,
        start: &str,
    ) -> SessionResult<Vec<Entry>> {
        validate_limit(query.limit)?;
        let order = query.order.unwrap_or_default();
        let path = match order {
            // Oldest-first must inspect the complete ancestry before applying
            // its forward-order boundary; walking backward with bounds here
            // chooses the wrong segment of the branch.
            EntryOrder::OldestFirst => self.walk_to_root_owned(start, None)?,
            EntryOrder::NewestFirst => self.walk_to_root_owned(start, Some(bounds))?,
        };
        let mut results = Vec::new();
        match order {
            EntryOrder::OldestFirst => {
                for entry in path.into_iter().rev() {
                    if self.matches_entry_query(&entry, query) {
                        results.push(entry.clone());
                    }
                    let reached_bound = bounds.stop_at_id.as_deref() == Some(entry.id())
                        || bounds
                            .stop_at_type
                            .map(|t| t == entry.entry_type())
                            .unwrap_or(false);
                    if reached_bound || query.limit.map(|l| results.len() >= l).unwrap_or(false) {
                        break;
                    }
                }
            }
            EntryOrder::NewestFirst => {
                for entry in path {
                    if self.matches_entry_query(&entry, query) {
                        results.push(entry);
                    }
                    if query.limit.map(|l| results.len() >= l).unwrap_or(false) {
                        break;
                    }
                }
            }
        }
        Ok(results)
    }

    pub fn find_records(&self, query: &RecordQuery) -> SessionResult<Vec<LaneRecord>> {
        validate_limit(query.limit)?;
        let order = query.order.unwrap_or_default();
        let mut results = Vec::new();
        let iter: Box<dyn Iterator<Item = &LaneRecord>> = match order {
            EntryOrder::OldestFirst => Box::new(self.records.iter()),
            EntryOrder::NewestFirst => Box::new(self.records.iter().rev()),
        };
        for record in iter {
            if !self.matches_record_query(record, query) {
                continue;
            }
            results.push(record.clone());
            if let Some(limit) = query.limit {
                if results.len() >= limit {
                    break;
                }
            }
        }
        Ok(results)
    }

    pub fn find_open_operations(
        &self,
        lane: &str,
        limit: Option<usize>,
    ) -> SessionResult<Vec<OperationStartedRecord>> {
        validate_limit(limit)?;
        let mut open: Vec<OperationStartedRecord> = self
            .open_operations_by_lane
            .get(lane)
            .map(|m| m.values().rev().cloned().collect())
            .unwrap_or_default();
        if let Some(limit) = limit {
            open.truncate(limit);
        }
        Ok(open)
    }

    pub fn get_log(&self, options: &LogOptions) -> SessionResult<Vec<LogItem>> {
        validate_limit(options.limit)?;
        let mut results = Vec::new();
        for item in &self.log {
            if let Some(after) = options.after_seq {
                if item.seq() <= after {
                    continue;
                }
            }
            results.push(item.clone());
            if let Some(limit) = options.limit {
                if results.len() >= limit {
                    break;
                }
            }
        }
        Ok(results)
    }

    pub fn get_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn get_label(&self, id: &str) -> Option<&str> {
        self.labels.get(id).map(|s| s.as_str())
    }

    pub fn get_stats(&self) -> &SessionStats {
        &self.stats
    }

    /// Build the fork mutation list used by the append-only JSONL format.
    /// The wire log includes the main-lane pointer mutation for branch forks,
    /// since replay has no implicit way to restore that pointer.
    pub fn create_fork_mutations(
        &self,
        options: &ForkOptions,
    ) -> SessionResult<Vec<SessionMutation>> {
        let contents = self.fork_contents(options)?;
        let mut mutations = Vec::new();
        let mut sequence = 1u64;
        for source_entry in &contents.copied_entries {
            let mut forked = source_entry.clone();
            forked.base_mut().seq = sequence;
            mutations.push(SessionMutation::Entry {
                seq: sequence,
                timestamp: source_entry.base().timestamp,
                lane: None,
                entry: forked,
            });
            sequence += 1;
        }
        for pointer in &contents.lanes {
            mutations.push(SessionMutation::Lane {
                seq: sequence,
                lane: pointer.lane.clone(),
                leaf_id: pointer.leaf_id.clone(),
            });
            sequence += 1;
        }
        if let Some(name) = &self.name {
            mutations.push(SessionMutation::FactName {
                seq: sequence,
                name: Some(name.clone()),
            });
            sequence += 1;
        }
        for (target_id, label) in self.fork_labels(&contents.copied_entries) {
            mutations.push(SessionMutation::FactLabel {
                seq: sequence,
                target_id,
                label: Some(label),
            });
            sequence += 1;
        }
        Ok(mutations)
    }

    /// Construct canonical in-memory fork state. Imported entries have no
    /// activity accounting, and a branch's main pointer is initialized
    /// directly rather than added as an observable mutation. Tree forks retain
    /// lane mutations because each lane is part of the copied tree history.
    pub(crate) fn create_fork_state(&self, options: &ForkOptions) -> SessionResult<Self> {
        let contents = self.fork_contents(options)?;
        let mut forked = SessionState::new();
        let mut sequence = 1u64;
        for source_entry in &contents.copied_entries {
            let mut entry = source_entry.clone();
            entry.base_mut().seq = sequence;
            forked.apply_mutation(SessionMutation::Entry {
                seq: sequence,
                timestamp: source_entry.base().timestamp,
                lane: None,
                entry,
            })?;
            sequence += 1;
        }
        if contents.is_tree {
            for pointer in &contents.lanes {
                forked.apply_mutation(SessionMutation::Lane {
                    seq: sequence,
                    lane: pointer.lane.clone(),
                    leaf_id: pointer.leaf_id.clone(),
                })?;
                sequence += 1;
            }
        } else {
            let main = contents
                .lanes
                .iter()
                .find(|pointer| pointer.lane == "main")
                .ok_or_else(|| SessionError::invalid_entry("Fork has no main lane"))?;
            if let Some(leaf_id) = main.leaf_id.as_deref() {
                if !forked.entries_by_id.contains_key(leaf_id) {
                    return Err(SessionError::invalid_entry(format!(
                        "Fork main lane references missing entry {leaf_id}"
                    )));
                }
            }
            forked.lanes.insert(main.lane.clone(), main.leaf_id.clone());
        }
        if let Some(name) = &self.name {
            forked.apply_mutation(SessionMutation::FactName {
                seq: sequence,
                name: Some(name.clone()),
            })?;
            sequence += 1;
        }
        for (target_id, label) in self.fork_labels(&contents.copied_entries) {
            forked.apply_mutation(SessionMutation::FactLabel {
                seq: sequence,
                target_id,
                label: Some(label),
            })?;
            sequence += 1;
        }
        Ok(forked)
    }

    fn fork_contents(&self, options: &ForkOptions) -> SessionResult<ForkContents> {
        match options {
            ForkOptions::Tree => Ok(ForkContents {
                copied_entries: self.find_entries(&EntryQuery {
                    order: Some(EntryOrder::OldestFirst),
                    ..Default::default()
                })?,
                lanes: self.get_lanes(),
                is_tree: true,
            }),
            ForkOptions::Branch { entry_id, position } => {
                let selected = match entry_id {
                    Some(id) => Some(id.clone()),
                    None => self.require_lane("main")?,
                };
                let target_id = if let Some(selected_id) = selected {
                    let entry = self.get_entry(&selected_id).ok_or_else(|| {
                        SessionError::invalid_fork_target(format!(
                            "Fork target is not a message entry: {selected_id}"
                        ))
                    })?;
                    if !matches!(entry, Entry::Message(_)) {
                        return Err(SessionError::invalid_fork_target(format!(
                            "Fork target is not a message entry: {selected_id}"
                        )));
                    }
                    let position = position.unwrap_or(match entry_id {
                        Some(_) => ForkPosition::Before,
                        None => ForkPosition::At,
                    });
                    match position {
                        ForkPosition::At => Some(entry.id().to_string()),
                        ForkPosition::Before => entry.parent_id().map(str::to_string),
                    }
                } else {
                    None
                };
                let copied_entries = match target_id.as_deref() {
                    Some(target) => self.find_entries_on_branch(
                        &EntryQuery {
                            order: Some(EntryOrder::OldestFirst),
                            ..Default::default()
                        },
                        &BranchBounds::default(),
                        target,
                    )?,
                    None => Vec::new(),
                };
                Ok(ForkContents {
                    copied_entries,
                    lanes: vec![LanePointer {
                        lane: "main".to_string(),
                        leaf_id: target_id,
                    }],
                    is_tree: false,
                })
            }
        }
    }

    fn fork_labels(&self, copied_entries: &[Entry]) -> Vec<(String, String)> {
        let copied_ids: HashSet<&str> = copied_entries.iter().map(|entry| entry.id()).collect();
        Self::ordered_keys(&self.labels_order, self.labels.keys().cloned())
            .into_iter()
            .filter_map(|target_id| {
                if !copied_ids.contains(target_id.as_str()) {
                    return None;
                }
                self.labels
                    .get(&target_id)
                    .map(|label| (target_id, label.clone()))
            })
            .collect()
    }

    // ---- private helpers ----

    fn walk_to_root_owned(
        &self,
        start: &str,
        bounds: Option<&BranchBounds>,
    ) -> SessionResult<Vec<Entry>> {
        let mut path = Vec::new();
        let mut next_id = Some(start.to_string());
        let mut visited = HashSet::new();
        while let Some(id) = next_id.take() {
            if !visited.insert(id.clone()) {
                return Err(SessionError::invalid_entry(format!(
                    "Session branch contains a cycle at {id}"
                )));
            }
            let current = self.entries_by_id.get(&id).ok_or_else(|| {
                if path.is_empty() {
                    SessionError::not_found(format!("Entry not found: {id}"))
                } else {
                    SessionError::invalid_entry(format!(
                        "Session branch references missing ancestor {id}"
                    ))
                }
            })?;
            let reached_bound = bounds
                .map(|bounds| {
                    bounds.stop_at_id.as_deref() == Some(current.id())
                        || bounds
                            .stop_at_type
                            .map(|t| t == current.entry_type())
                            .unwrap_or(false)
                })
                .unwrap_or(false);
            let reached_root = current.parent_id().is_none();
            path.push(current.clone());
            if reached_bound || reached_root {
                break;
            }
            next_id = current.parent_id().map(str::to_string);
        }
        Ok(path)
    }

    fn matches_entry_query(&self, entry: &Entry, query: &EntryQuery) -> bool {
        let type_ok = query
            .entry_type
            .map(|t| t == entry.entry_type())
            .unwrap_or(true);
        let custom_ok = query
            .custom_type
            .as_deref()
            .map(|ct| {
                if let Entry::Custom(c) = entry {
                    c.custom_type == ct
                } else {
                    false
                }
            })
            .unwrap_or(true);
        let cursor_ok = query
            .cursor
            .map(|c| {
                let after = c.after_seq;
                match query.order.unwrap_or_default() {
                    EntryOrder::OldestFirst => entry.seq() > after,
                    EntryOrder::NewestFirst => entry.seq() < after,
                }
            })
            .unwrap_or(true);
        type_ok && custom_ok && cursor_ok
    }

    fn matches_record_query(&self, record: &LaneRecord, query: &RecordQuery) -> bool {
        let lane_ok = query
            .lane
            .as_deref()
            .map(|l| record.lane() == l)
            .unwrap_or(true);
        let type_ok = query
            .record_type
            .map(|t| t == record.record_type())
            .unwrap_or(true);
        let run_ok = query
            .run_id
            .as_deref()
            .map(|run| match record {
                LaneRecord::OperationStarted(s) => s.base.id == run,
                _ => record.run_id().map(|r| r == run).unwrap_or(false),
            })
            .unwrap_or(true);
        let kind_ok = query
            .operation_kind
            .map(|k| {
                matches!(record,
                    LaneRecord::OperationStarted(s) if s.intent.kind() == k)
            })
            .unwrap_or(true);
        let after_ok = query
            .after_seq
            .map(|after| record.seq() > after)
            .unwrap_or(true);
        lane_ok && type_ok && run_ok && kind_ok && after_ok
    }
}

struct ForkContents {
    copied_entries: Vec<Entry>,
    lanes: Vec<LanePointer>,
    is_tree: bool,
}

/// What `apply_mutation` returned — mirrors the TS pattern where each mutation
/// variant updates different state.
#[derive(Debug, Clone)]
pub enum ApplyOutcome {
    Entry(Entry),
    Record(LaneRecord),
    Lane,
    Fact,
}

/// Outcome of a fork: the metadata to stamp on the new session.
#[derive(Debug, Clone)]
pub struct ForkResult {
    pub metadata: SessionMetadata,
    pub mutations: Vec<SessionMutation>,
}

// ---- helpers ----

fn validate_limit(limit: Option<usize>) -> SessionResult<()> {
    if let Some(l) = limit {
        if l == 0 {
            return Err(SessionError::invalid_query(
                "limit must be a positive integer",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(dead_code)] // state introspection helpers for upcoming session_state tests
impl SessionState {
    pub(crate) fn lanes(&self) -> &BTreeMap<String, Option<String>> {
        &self.lanes
    }
    pub(crate) fn entries(&self) -> &[Entry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::types::{provisioned_into_entry, ProvisionedEntry, ProvisionedKind};
    use ice_agent::message::AgentMessage;
    use ice_ai::types::UserMessage;

    fn message(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage::new(text, 1))
    }

    fn append(state: &mut SessionState, id: &str, kind: ProvisionedKind) {
        let seq = state.next_sequence();
        let parent_id = state.require_lane("main").expect("main lane exists");
        let entry = provisioned_into_entry(
            ProvisionedEntry {
                id: id.to_string(),
                kind,
            },
            seq,
            parent_id,
            seq as i64,
        );
        state
            .apply_mutation(SessionMutation::Entry {
                seq,
                timestamp: seq as i64,
                lane: Some("main".to_string()),
                entry,
            })
            .expect("entry appends to main");
    }

    fn branch_fixture() -> SessionState {
        let mut state = SessionState::new();
        append(
            &mut state,
            "root",
            ProvisionedKind::Message {
                message: message("root"),
                terminate: None,
            },
        );
        append(
            &mut state,
            "old-note",
            ProvisionedKind::Custom {
                custom_type: "note".to_string(),
                data: None,
            },
        );
        append(
            &mut state,
            "compact",
            ProvisionedKind::Compaction {
                summary: "summary".to_string(),
                retained_tail: Vec::new(),
                tokens_before: 0,
                details: None,
                usage: None,
            },
        );
        append(
            &mut state,
            "new-note",
            ProvisionedKind::Custom {
                custom_type: "note".to_string(),
                data: None,
            },
        );
        append(
            &mut state,
            "tail",
            ProvisionedKind::Message {
                message: message("tail"),
                terminate: None,
            },
        );
        state
    }

    fn ids(entries: Vec<Entry>) -> Vec<String> {
        entries
            .into_iter()
            .map(|entry| entry.id().to_string())
            .collect()
    }

    #[test]
    fn oldest_first_branch_bounds_are_applied_during_forward_scan() {
        let state = branch_fixture();
        let entries = state
            .find_entries_on_branch(
                &EntryQuery {
                    order: Some(EntryOrder::OldestFirst),
                    ..EntryQuery::default()
                },
                &BranchBounds {
                    stop_at_type: Some("custom"),
                    ..BranchBounds::default()
                },
                "tail",
            )
            .expect("branch query succeeds");
        assert_eq!(ids(entries), ["root", "old-note"]);
    }

    #[test]
    fn branch_bounds_filters_cursors_and_limits_are_directional() {
        let state = branch_fixture();

        let oldest_filtered_bound = state
            .find_entries_on_branch(
                &EntryQuery {
                    entry_type: Some("message"),
                    order: Some(EntryOrder::OldestFirst),
                    ..EntryQuery::default()
                },
                &BranchBounds {
                    stop_at_id: Some("new-note".to_string()),
                    ..BranchBounds::default()
                },
                "tail",
            )
            .expect("oldest filtered query succeeds");
        assert_eq!(ids(oldest_filtered_bound), ["root"]);

        let newest_filtered_bound = state
            .find_entries_on_branch(
                &EntryQuery {
                    custom_type: Some("note".to_string()),
                    ..EntryQuery::default()
                },
                &BranchBounds {
                    stop_at_id: Some("tail".to_string()),
                    ..BranchBounds::default()
                },
                "tail",
            )
            .expect("newest filtered query succeeds");
        assert!(newest_filtered_bound.is_empty());

        let newest_type_bound = state
            .find_entries_on_branch(
                &EntryQuery::default(),
                &BranchBounds {
                    stop_at_type: Some("custom"),
                    ..BranchBounds::default()
                },
                "tail",
            )
            .expect("newest type-bound query succeeds");
        assert_eq!(ids(newest_type_bound), ["tail", "new-note"]);

        let oldest_cursor = state
            .find_entries_on_branch(
                &EntryQuery {
                    custom_type: Some("note".to_string()),
                    order: Some(EntryOrder::OldestFirst),
                    cursor: Some(crate::session::types::EntryCursor { after_seq: 2 }),
                    ..EntryQuery::default()
                },
                &BranchBounds {
                    stop_at_id: Some("tail".to_string()),
                    ..BranchBounds::default()
                },
                "tail",
            )
            .expect("oldest cursor query succeeds");
        assert_eq!(ids(oldest_cursor), ["new-note"]);

        let newest_cursor = state
            .find_entries_on_branch(
                &EntryQuery {
                    custom_type: Some("note".to_string()),
                    cursor: Some(crate::session::types::EntryCursor { after_seq: 4 }),
                    ..EntryQuery::default()
                },
                &BranchBounds {
                    stop_at_id: Some("root".to_string()),
                    ..BranchBounds::default()
                },
                "tail",
            )
            .expect("newest cursor query succeeds");
        assert_eq!(ids(newest_cursor), ["old-note"]);

        let oldest_maximum_cursor = state
            .find_entries_on_branch(
                &EntryQuery {
                    order: Some(EntryOrder::OldestFirst),
                    cursor: Some(crate::session::types::EntryCursor {
                        after_seq: u64::MAX,
                    }),
                    ..EntryQuery::default()
                },
                &BranchBounds::default(),
                "tail",
            )
            .expect("oldest maximum cursor is a valid empty page");
        assert!(oldest_maximum_cursor.is_empty());

        let newest_maximum_cursor = state
            .find_entries_on_branch(
                &EntryQuery {
                    order: Some(EntryOrder::NewestFirst),
                    cursor: Some(crate::session::types::EntryCursor {
                        after_seq: u64::MAX,
                    }),
                    ..EntryQuery::default()
                },
                &BranchBounds::default(),
                "tail",
            )
            .expect("newest maximum cursor is valid");
        assert_eq!(
            ids(newest_maximum_cursor),
            ["tail", "new-note", "compact", "old-note", "root"]
        );

        let limited_oldest = state
            .find_entries_on_branch(
                &EntryQuery {
                    custom_type: Some("note".to_string()),
                    order: Some(EntryOrder::OldestFirst),
                    limit: Some(1),
                    ..EntryQuery::default()
                },
                &BranchBounds::default(),
                "tail",
            )
            .expect("limited oldest query succeeds");
        assert_eq!(ids(limited_oldest), ["old-note"]);

        let limited_newest = state
            .find_entries_on_branch(
                &EntryQuery {
                    custom_type: Some("note".to_string()),
                    limit: Some(1),
                    ..EntryQuery::default()
                },
                &BranchBounds::default(),
                "tail",
            )
            .expect("limited newest query succeeds");
        assert_eq!(ids(limited_newest), ["new-note"]);
    }

    #[test]
    fn branch_start_and_broken_ancestor_errors_remain_distinct() {
        let mut state = branch_fixture();
        let missing_start = state.find_entries_on_branch(
            &EntryQuery::default(),
            &BranchBounds::default(),
            "missing",
        );
        assert_eq!(
            missing_start.unwrap_err().code,
            crate::error::SessionErrorCode::NotFound
        );

        let mut tail = state.get_entry("tail").expect("tail exists").clone();
        tail.base_mut().parent_id = Some("missing-ancestor".to_string());
        state.entries_by_id.insert("tail".to_string(), tail);
        let missing_parent =
            state.find_entries_on_branch(&EntryQuery::default(), &BranchBounds::default(), "tail");
        assert_eq!(
            missing_parent.unwrap_err().code,
            crate::error::SessionErrorCode::InvalidEntry
        );

        let mut tail = state.get_entry("tail").expect("tail exists").clone();
        tail.base_mut().parent_id = Some("root".to_string());
        state.entries_by_id.insert("tail".to_string(), tail);
        let mut root = state.get_entry("root").expect("root exists").clone();
        root.base_mut().parent_id = Some("tail".to_string());
        state.entries_by_id.insert("root".to_string(), root);
        let cycle =
            state.find_entries_on_branch(&EntryQuery::default(), &BranchBounds::default(), "tail");
        assert_eq!(
            cycle.unwrap_err().code,
            crate::error::SessionErrorCode::InvalidEntry
        );
    }

    #[test]
    fn lanes_keep_creation_order_and_updates_do_not_reorder() {
        let mut state = SessionState::new();
        for (seq, lane) in [(1, "zeta"), (2, "alpha")] {
            state
                .apply_mutation(SessionMutation::Lane {
                    seq,
                    lane: lane.to_string(),
                    leaf_id: None,
                })
                .expect("lane mutation applies");
        }
        state
            .apply_mutation(SessionMutation::Lane {
                seq: 3,
                lane: "zeta".to_string(),
                leaf_id: None,
            })
            .expect("existing lane move applies");
        let lanes: Vec<_> = state.get_lanes().into_iter().map(|p| p.lane).collect();
        assert_eq!(lanes, ["main", "zeta", "alpha"]);
    }

    #[test]
    fn fork_targets_distinguish_implicit_explicit_before_and_at() {
        let mut state = SessionState::new();
        for (id, kind) in [
            (
                "custom",
                ProvisionedKind::Custom {
                    custom_type: "note".to_string(),
                    data: None,
                },
            ),
            (
                "message-a",
                ProvisionedKind::Message {
                    message: message("a"),
                    terminate: None,
                },
            ),
            (
                "message-b",
                ProvisionedKind::Message {
                    message: message("b"),
                    terminate: None,
                },
            ),
        ] {
            append(&mut state, id, kind);
        }
        let fork_entries_and_leaf = |options: ForkOptions| {
            let mutations = state
                .create_fork_mutations(&options)
                .expect("fork mutation list");
            let entries = mutations
                .iter()
                .filter_map(|mutation| match mutation {
                    SessionMutation::Entry { entry, .. } => Some(entry.id().to_string()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let leaf = mutations.iter().find_map(|mutation| match mutation {
                SessionMutation::Lane { lane, leaf_id, .. } if lane == "main" => {
                    Some(leaf_id.clone())
                }
                _ => None,
            });
            (entries, leaf)
        };
        assert_eq!(
            fork_entries_and_leaf(ForkOptions::default()),
            (
                vec![
                    "custom".to_string(),
                    "message-a".to_string(),
                    "message-b".to_string(),
                ],
                Some(Some("message-b".to_string()))
            )
        );
        assert_eq!(
            fork_entries_and_leaf(ForkOptions::Branch {
                entry_id: Some("message-b".to_string()),
                position: None,
            }),
            (
                vec!["custom".to_string(), "message-a".to_string()],
                Some(Some("message-a".to_string()))
            )
        );
        assert_eq!(
            fork_entries_and_leaf(ForkOptions::Branch {
                entry_id: Some("message-b".to_string()),
                position: Some(ForkPosition::At),
            }),
            (
                vec![
                    "custom".to_string(),
                    "message-a".to_string(),
                    "message-b".to_string(),
                ],
                Some(Some("message-b".to_string()))
            )
        );
        let missing = state
            .create_fork_mutations(&ForkOptions::Branch {
                entry_id: Some("missing".to_string()),
                position: None,
            })
            .expect_err("missing target is rejected");
        assert_eq!(
            missing.code,
            crate::error::SessionErrorCode::InvalidForkTarget
        );
        let non_message = state
            .create_fork_mutations(&ForkOptions::Branch {
                entry_id: Some("custom".to_string()),
                position: None,
            })
            .expect_err("non-message target is rejected");
        assert_eq!(
            non_message.code,
            crate::error::SessionErrorCode::InvalidForkTarget
        );
    }

    #[test]
    fn legacy_state_order_fallback_is_deterministic() {
        let mut state = SessionState::new();
        for (seq, lane) in [(1, "zeta"), (2, "alpha")] {
            state
                .apply_mutation(SessionMutation::Lane {
                    seq,
                    lane: lane.to_string(),
                    leaf_id: None,
                })
                .expect("lane mutation applies");
        }
        for id in ["entry-z", "entry-a"] {
            append(
                &mut state,
                id,
                ProvisionedKind::Message {
                    message: message(id),
                    terminate: None,
                },
            );
        }
        for target_id in ["entry-z", "entry-a"] {
            let seq = state.next_sequence();
            state
                .apply_mutation(SessionMutation::FactLabel {
                    seq,
                    target_id: target_id.to_string(),
                    label: Some(target_id.to_string()),
                })
                .expect("label mutation applies");
        }
        let mut json = serde_json::to_value(state).expect("serialize state");
        if let serde_json::Value::Object(object) = &mut json {
            object.remove("lane_order");
            object.remove("labels_order");
        }
        let restored: SessionState = serde_json::from_value(json).expect("legacy state decodes");
        let lanes: Vec<_> = restored.get_lanes().into_iter().map(|p| p.lane).collect();
        assert_eq!(lanes, ["alpha", "main", "zeta"]);
        let labels: Vec<_> = restored
            .create_fork_mutations(&ForkOptions::default())
            .expect("legacy fork mutations")
            .into_iter()
            .filter_map(|mutation| match mutation {
                SessionMutation::FactLabel { target_id, .. } => Some(target_id),
                _ => None,
            })
            .collect();
        assert_eq!(labels, ["entry-a", "entry-z"]);
    }

    #[test]
    fn fork_label_facts_keep_source_insertion_order_and_reinsert_moves_to_end() {
        let mut state = SessionState::new();
        for id in ["entry-a", "entry-b", "entry-c"] {
            append(
                &mut state,
                id,
                ProvisionedKind::Message {
                    message: message(id),
                    terminate: None,
                },
            );
        }
        fn set_label(state: &mut SessionState, target_id: &str, label: Option<&str>) {
            let seq = state.next_sequence();
            state
                .apply_mutation(SessionMutation::FactLabel {
                    seq,
                    target_id: target_id.to_string(),
                    label: label.map(str::to_string),
                })
                .expect("label mutation applies");
        }
        set_label(&mut state, "entry-c", Some("c"));
        set_label(&mut state, "entry-a", Some("a"));
        set_label(&mut state, "entry-c", Some("updated-c"));

        let labels = |state: &SessionState| {
            state
                .create_fork_mutations(&ForkOptions::default())
                .expect("fork mutations build")
                .into_iter()
                .filter_map(|mutation| match mutation {
                    SessionMutation::FactLabel { target_id, .. } => Some(target_id),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(labels(&state), ["entry-c", "entry-a"]);

        set_label(&mut state, "entry-c", None);
        set_label(&mut state, "entry-c", Some("readded-c"));
        assert_eq!(labels(&state), ["entry-a", "entry-c"]);
    }
}
