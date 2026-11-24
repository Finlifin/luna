//! Unified local-dependency scheduler and watcher registry for elaboration.
//!
//! Goals suspend on versioned dependency keys via generation-checked wait
//! tickets. When dependencies publish updates, merge via union-find, or fail,
//! watchers are forwarded and woken deterministically without re-entering the
//! caller or leaving permanent waits behind.

use nir::DeclarationId;
use nir::program as n;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Generation-tagged reference to a goal slot. Reusing a freed slot advances
/// its generation so stale tickets from completed, cancelled, or rolled-back
/// goals can never wake a new goal occupying the same slot index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::program) struct GoalRef {
    pub slot: usize,
    pub generation: u64,
}

impl std::fmt::Display for GoalRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(goal {} {})", self.slot, self.generation)
    }
}

/// Canonical identity of a local dependency observed by a goal.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::program) enum DependencyKey {
    Slot(usize),
    Local(n::LocalId),
    Store(usize),
    Declaration(DeclarationId),
    Goal(GoalRef),
    Tag(u64),
}

impl std::fmt::Display for DependencyKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Slot(slot) => write!(f, "(slot {slot})"),
            Self::Local(id) => write!(f, "(local {id})"),
            Self::Store(id) => write!(f, "(store {id})"),
            Self::Declaration(id) => write!(f, "(declaration {id})"),
            Self::Goal(goal) => write!(f, "(subgoal {} {})", goal.slot, goal.generation),
            Self::Tag(id) => write!(f, "(tag {id})"),
        }
    }
}

/// Ticket returned when a goal registers a watch on a dependency at an
/// observed version.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::program) struct WaitTicket {
    pub goal: GoalRef,
    pub dependency: DependencyKey,
    pub observed_version: u64,
}

impl std::fmt::Display for WaitTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(wait-ticket {} {} (observed-version {}))",
            self.goal, self.dependency, self.observed_version
        )
    }
}

/// Structured reason explaining why a local goal failed or was cancelled.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::program) enum FailureReason {
    ProducerFailed {
        dependency: DependencyKey,
        detail: String,
    },
    Cycle {
        path: Vec<GoalRef>,
    },
    Unresolved {
        dependency: DependencyKey,
    },
    Cancelled {
        detail: String,
    },
}

impl std::fmt::Display for FailureReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProducerFailed { dependency, detail } => {
                write!(f, "(producer-failed {dependency} (detail {detail:?}))")
            }
            Self::Cycle { path } => {
                f.write_str("(cycle (path")?;
                for goal in path {
                    write!(f, " {goal}")?;
                }
                f.write_str("))")
            }
            Self::Unresolved { dependency } => write!(f, "(unresolved {dependency})"),
            Self::Cancelled { detail } => write!(f, "(cancelled {detail:?})"),
        }
    }
}

/// Lifecycle state of a goal stored in a slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::program) enum GoalState {
    Runnable,
    Waiting(BTreeSet<DependencyKey>),
    Completed,
    Failed(FailureReason),
    Cancelled(FailureReason),
}

impl std::fmt::Display for GoalState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Runnable => f.write_str("(state runnable)"),
            Self::Waiting(keys) => {
                f.write_str("(state (waiting")?;
                for key in keys {
                    write!(f, " {key}")?;
                }
                f.write_str("))")
            }
            Self::Completed => f.write_str("(state completed)"),
            Self::Failed(reason) => write!(f, "(state (failed {reason}))"),
            Self::Cancelled(reason) => write!(f, "(state (cancelled {reason}))"),
        }
    }
}

/// Metadata stored for each allocated or previously allocated goal slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::program) struct Goal {
    pub reference: GoalRef,
    pub state: GoalState,
    pub watches: BTreeSet<DependencyKey>,
    pub produces: BTreeSet<DependencyKey>,
    pub label: Option<String>,
}

impl std::fmt::Display for Goal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(goal-entry {} {}", self.reference, self.state)?;
        if let Some(label) = &self.label {
            write!(f, " (label {label:?})")?;
        }
        if !self.produces.is_empty() {
            f.write_str(" (produces")?;
            for key in &self.produces {
                write!(f, " {key}")?;
            }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ProducerStatus {
    Pending,
    Produced,
    Failed(String),
}

impl Default for ProducerStatus {
    fn default() -> Self {
        Self::Pending
    }
}

impl std::fmt::Display for ProducerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => f.write_str("pending"),
            Self::Produced => f.write_str("produced"),
            Self::Failed(detail) => write!(f, "(failed {detail:?})"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DependencyEntry {
    version: u64,
    status: ProducerStatus,
    producer: Option<GoalRef>,
    watchers: BTreeSet<GoalRef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SlotRecord {
    generation: u64,
    occupied: bool,
    state: GoalState,
    watches: BTreeSet<DependencyKey>,
    produces: BTreeSet<DependencyKey>,
    label: Option<String>,
}

impl SlotRecord {
    fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            GoalState::Completed | GoalState::Failed(_) | GoalState::Cancelled(_)
        )
    }

    fn as_goal(&self, slot: usize) -> Goal {
        Goal {
            reference: GoalRef {
                slot,
                generation: self.generation,
            },
            state: self.state.clone(),
            watches: self.watches.clone(),
            produces: self.produces.clone(),
            label: self.label.clone(),
        }
    }
}

/// Summary produced when finishing a local dependency region.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::program) struct FinishReport {
    pub completed: usize,
    pub failed: Vec<(GoalRef, FailureReason)>,
    pub cycles_cancelled: Vec<Vec<GoalRef>>,
    pub unresolved_cancelled: Vec<(GoalRef, DependencyKey)>,
    pub runnable_remaining: usize,
}

impl FinishReport {
    pub(in crate::program) fn is_clean(&self) -> bool {
        self.failed.is_empty()
            && self.cycles_cancelled.is_empty()
            && self.unresolved_cancelled.is_empty()
            && self.runnable_remaining == 0
    }
}

impl std::fmt::Display for FinishReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(finish-report (completed {}) (runnable-remaining {}) (failed",
            self.completed, self.runnable_remaining
        )?;
        for (goal, reason) in &self.failed {
            write!(f, " ({goal} {reason})")?;
        }
        f.write_str(") (cycles")?;
        for cycle in &self.cycles_cancelled {
            f.write_str(" (cycle")?;
            for goal in cycle {
                write!(f, " {goal}")?;
            }
            f.write_str(")")?;
        }
        f.write_str(") (unresolved")?;
        for (goal, dep) in &self.unresolved_cancelled {
            write!(f, " ({goal} {dep})")?;
        }
        f.write_str("))")
    }
}

/// Rollback checkpoint for speculative branches or nested probes.
/// Rolling back restores earlier goals and watches while strictly advancing
/// slot generations for any slot touched after the checkpoint so stale
/// wait tickets from the discarded branch can never wake reused slots.
#[derive(Clone, Debug)]
pub(in crate::program) struct Checkpoint {
    slots: Vec<SlotRecord>,
    free_slots: BTreeSet<usize>,
    parent: BTreeMap<DependencyKey, DependencyKey>,
    dependencies: BTreeMap<DependencyKey, DependencyEntry>,
    ready_queue: VecDeque<GoalRef>,
    ready_set: BTreeSet<GoalRef>,
    completed_count: usize,
    failed_goals: BTreeMap<GoalRef, FailureReason>,
}

/// Deterministic local-dependency scheduler using ordered `BTreeMap`/`BTreeSet`
/// structures, generation-checked `WaitTicket`s, union-find watcher forwarding,
/// and topological cycle/failure cancellation.
#[derive(Clone, Debug, Default)]
pub(in crate::program) struct WatcherRegistry {
    slots: Vec<SlotRecord>,
    free_slots: BTreeSet<usize>,
    parent: BTreeMap<DependencyKey, DependencyKey>,
    dependencies: BTreeMap<DependencyKey, DependencyEntry>,
    ready_queue: VecDeque<GoalRef>,
    ready_set: BTreeSet<GoalRef>,
    completed_count: usize,
    failed_goals: BTreeMap<GoalRef, FailureReason>,
}

impl WatcherRegistry {
    pub(in crate::program) fn new() -> Self {
        Self::default()
    }

    #[allow(dead_code)]
    pub(in crate::program) fn is_empty(&self) -> bool {
        self.slots.iter().all(|slot| !slot.occupied)
            && self.ready_queue.is_empty()
            && self.dependencies.is_empty()
    }

    pub(in crate::program) fn has_activity(&self) -> bool {
        !self.dependencies.is_empty() || !self.failed_goals.is_empty() || !self.parent.is_empty()
    }

    /// Returns whether `goal` refers to a currently occupied, matching-generation slot.
    pub(in crate::program) fn is_live(&self, goal: GoalRef) -> bool {
        self.slots
            .get(goal.slot)
            .is_some_and(|slot| slot.occupied && slot.generation == goal.generation)
    }

    #[allow(dead_code)]
    pub(in crate::program) fn goal(&self, goal: GoalRef) -> Option<Goal> {
        self.is_live(goal)
            .then(|| self.slots[goal.slot].as_goal(goal.slot))
    }

    pub(in crate::program) fn live_goal_in_slot(&self, slot: usize) -> Option<GoalRef> {
        self.slots.get(slot).and_then(|record| {
            record.occupied.then_some(GoalRef {
                slot,
                generation: record.generation,
            })
        })
    }

    #[allow(dead_code)]
    pub(in crate::program) fn goal_failure(&self, goal: GoalRef) -> Option<&FailureReason> {
        self.failed_goals.get(&goal)
    }

    pub(in crate::program) fn spawn_goal(&mut self, label: Option<String>) -> GoalRef {
        let goal = self.allocate_slot(GoalState::Runnable, label);
        self.enqueue_ready(goal);
        goal
    }

    pub(in crate::program) fn spawn_waiting_goal(&mut self, label: Option<String>) -> GoalRef {
        self.allocate_slot(GoalState::Waiting(BTreeSet::new()), label)
    }

    fn allocate_slot(&mut self, state: GoalState, label: Option<String>) -> GoalRef {
        if let Some(slot) = self.free_slots.pop_first() {
            let record = &mut self.slots[slot];
            let generation = record.generation;
            record.occupied = true;
            record.state = state;
            record.watches.clear();
            record.produces.clear();
            record.label = label;
            GoalRef { slot, generation }
        } else {
            let slot = self.slots.len();
            self.slots.push(SlotRecord {
                generation: 0,
                occupied: true,
                state,
                watches: BTreeSet::new(),
                produces: BTreeSet::new(),
                label,
            });
            GoalRef {
                slot,
                generation: 0,
            }
        }
    }

    fn enqueue_ready(&mut self, goal: GoalRef) -> bool {
        if !self.is_live(goal) || self.slots[goal.slot].is_terminal() {
            return false;
        }
        if self.ready_set.insert(goal) {
            self.ready_queue.push_back(goal);
            true
        } else {
            false
        }
    }

    /// Pop the next runnable goal. When a goal resumes, clear its prior watch
    /// registrations so it re-observes its premises cleanly without stale watches.
    pub(in crate::program) fn pop_ready(&mut self) -> Option<GoalRef> {
        while let Some(goal) = self.ready_queue.pop_front() {
            self.ready_set.remove(&goal);
            if !self.is_live(goal) || self.slots[goal.slot].is_terminal() {
                continue;
            }
            self.clear_goal_watches(goal);
            self.slots[goal.slot].state = GoalState::Runnable;
            return Some(goal);
        }
        None
    }

    #[allow(dead_code)]
    pub(in crate::program) fn ready_len(&self) -> usize {
        self.ready_set.len()
    }

    /// Find the canonical representative of `key` with iterative path compression.
    pub(in crate::program) fn find_representative(&mut self, key: &DependencyKey) -> DependencyKey {
        let mut current = key.clone();
        let mut trail = Vec::new();
        while let Some(next) = self.parent.get(&current) {
            if *next == current {
                break;
            }
            trail.push(current);
            current = next.clone();
        }
        for node in trail {
            self.parent.insert(node, current.clone());
        }
        current
    }

    fn peek_representative_in(
        parent: &BTreeMap<DependencyKey, DependencyKey>,
        key: &DependencyKey,
    ) -> DependencyKey {
        let mut current = key;
        while let Some(next) = parent.get(current) {
            if next == current {
                break;
            }
            current = next;
        }
        current.clone()
    }

    fn peek_representative(&self, key: &DependencyKey) -> DependencyKey {
        Self::peek_representative_in(&self.parent, key)
    }

    /// Record `producer` as the active producer goal for `key`, used for
    /// cycle detection and transitive producer-failure propagation.
    pub(in crate::program) fn bind_producer(
        &mut self,
        key: DependencyKey,
        producer: GoalRef,
    ) -> bool {
        if !self.is_live(producer) || self.slots[producer.slot].is_terminal() {
            return false;
        }
        let rep = self.find_representative(&key);
        let entry = self.dependencies.entry(rep.clone()).or_default();
        entry.producer = Some(producer);
        self.slots[producer.slot].produces.insert(rep);
        true
    }

    /// Observe the canonical representative and version of `key`.
    pub(in crate::program) fn observe_version(
        &mut self,
        key: &DependencyKey,
    ) -> (DependencyKey, u64) {
        let rep = self.find_representative(key);
        let version = self.dependencies.entry(rep.clone()).or_default().version;
        (rep, version)
    }

    /// Register `goal` as a watcher of `key` after observing `observed_version`.
    /// Immediately re-checks the version and enqueues `goal` if `key` changed
    /// between observation and registration. If the producer already failed,
    /// fails/wakes `goal` immediately instead of leaving a permanent wait.
    pub(in crate::program) fn register_watch(
        &mut self,
        goal: GoalRef,
        key: DependencyKey,
        observed_version: u64,
    ) -> Result<WaitTicket, FailureReason> {
        if !self.is_live(goal) {
            return Err(FailureReason::Cancelled {
                detail: "stale goal slot cannot register watch".to_string(),
            });
        }
        if let Some(reason) = self.failed_goals.get(&goal) {
            return Err(reason.clone());
        }
        if self.slots[goal.slot].is_terminal() {
            return Err(FailureReason::Cancelled {
                detail: "terminal goal cannot register watch".to_string(),
            });
        }

        let rep = self.find_representative(&key);
        let (version, failed_detail) = {
            let entry = self.dependencies.entry(rep.clone()).or_default();
            let failed_detail = match &entry.status {
                ProducerStatus::Failed(detail) => Some(detail.clone()),
                ProducerStatus::Pending | ProducerStatus::Produced => None,
            };
            (entry.version, failed_detail)
        };

        if let Some(detail) = failed_detail {
            let reason = FailureReason::ProducerFailed {
                dependency: rep,
                detail,
            };
            self.fail_goal_with_reason(goal, reason.clone());
            return Err(reason);
        }

        self.dependencies
            .entry(rep.clone())
            .or_default()
            .watchers
            .insert(goal);
        self.slots[goal.slot].watches.insert(rep.clone());
        match &mut self.slots[goal.slot].state {
            GoalState::Waiting(keys) => {
                keys.insert(rep.clone());
            }
            state @ GoalState::Runnable => {
                *state = GoalState::Waiting(BTreeSet::from([rep.clone()]));
            }
            _ => {}
        }

        // Version re-check after registration: if updated between observe and
        // register, enqueue immediately with generation-keyed deduplication.
        if version != observed_version {
            self.enqueue_ready(goal);
        }

        Ok(WaitTicket {
            goal,
            dependency: rep,
            observed_version,
        })
    }

    /// Wake the goal referenced by `ticket` if and only if its slot generation
    /// still matches and the goal is still live and non-terminal.
    pub(in crate::program) fn wake_ticket(&mut self, ticket: &WaitTicket) -> bool {
        if !self.is_live(ticket.goal) || self.slots[ticket.goal.slot].is_terminal() {
            return false;
        }
        if matches!(self.slots[ticket.goal.slot].state, GoalState::Waiting(_)) {
            self.slots[ticket.goal.slot].state = GoalState::Runnable;
        }
        self.enqueue_ready(ticket.goal)
    }

    /// Publish a dependency update, bumping its version and enqueueing all
    /// live observers without re-entering the caller.
    pub(in crate::program) fn publish_update(&mut self, key: &DependencyKey) -> usize {
        let rep = self.find_representative(key);
        let watchers = {
            let entry = self.dependencies.entry(rep).or_default();
            entry.version = entry.version.checked_add(1).unwrap_or(u64::MAX);
            if matches!(entry.status, ProducerStatus::Pending) {
                entry.status = ProducerStatus::Produced;
            }
            std::mem::take(&mut entry.watchers)
        };
        let mut woken = 0usize;
        for goal in watchers {
            if !self.is_live(goal) || self.slots[goal.slot].is_terminal() {
                continue;
            }
            if matches!(self.slots[goal.slot].state, GoalState::Waiting(_)) {
                self.slots[goal.slot].state = GoalState::Runnable;
            }
            if self.enqueue_ready(goal) {
                woken += 1;
            }
        }
        woken
    }

    /// Merge `source` into `target` in the union-find structure, forwarding all
    /// watchers and producer status onto `target` and waking all observers to
    /// re-observe under the merged representative.
    #[allow(dead_code)]
    pub(in crate::program) fn merge_dependencies(
        &mut self,
        source: DependencyKey,
        target: DependencyKey,
    ) -> DependencyKey {
        let rep_src = self.find_representative(&source);
        let rep_dst = self.find_representative(&target);
        if rep_src == rep_dst {
            return rep_dst;
        }

        self.parent.insert(rep_src.clone(), rep_dst.clone());
        let src_entry = self.dependencies.remove(&rep_src).unwrap_or_default();
        let mut forwarded_watchers = Vec::new();
        for watcher in src_entry.watchers {
            if self.is_live(watcher) && !self.slots[watcher.slot].is_terminal() {
                let slot = &mut self.slots[watcher.slot];
                slot.watches.remove(&rep_src);
                slot.watches.insert(rep_dst.clone());
                if let GoalState::Waiting(keys) = &mut slot.state {
                    keys.remove(&rep_src);
                    keys.insert(rep_dst.clone());
                }
                forwarded_watchers.push(watcher);
            }
        }
        let dst_producer = self
            .dependencies
            .get(&rep_dst)
            .and_then(|entry| entry.producer)
            .or(src_entry.producer);
        if let Some(producer) = dst_producer
            && self.is_live(producer)
        {
            let slot = &mut self.slots[producer.slot];
            slot.produces.remove(&rep_src);
            slot.produces.insert(rep_dst.clone());
        }

        let (merged_failure, all_watchers) = {
            let dst_entry = self.dependencies.entry(rep_dst.clone()).or_default();
            dst_entry.version = dst_entry
                .version
                .max(src_entry.version)
                .checked_add(1)
                .unwrap_or(u64::MAX);

            match (&dst_entry.status, src_entry.status) {
                (ProducerStatus::Failed(_), _) => {}
                (_, ProducerStatus::Failed(detail)) => {
                    dst_entry.status = ProducerStatus::Failed(detail);
                }
                (ProducerStatus::Pending, ProducerStatus::Produced) => {
                    dst_entry.status = ProducerStatus::Produced;
                }
                _ => {}
            }

            if dst_entry.producer.is_none() {
                dst_entry.producer = dst_producer;
            }

            for watcher in forwarded_watchers {
                dst_entry.watchers.insert(watcher);
            }

            let failure = match &dst_entry.status {
                ProducerStatus::Failed(detail) => Some(detail.clone()),
                _ => None,
            };
            let watchers = if failure.is_some() {
                std::mem::take(&mut dst_entry.watchers)
            } else {
                dst_entry.watchers.clone()
            };
            (failure, watchers)
        };

        if let Some(detail) = merged_failure {
            let mut transitive = Vec::new();
            for watcher in all_watchers {
                if self.is_live(watcher) && !self.slots[watcher.slot].is_terminal() {
                    let reason = FailureReason::ProducerFailed {
                        dependency: rep_dst.clone(),
                        detail: detail.clone(),
                    };
                    transitive.extend(self.fail_goal_with_reason(watcher, reason));
                }
            }
            self.propagate_failed_keys(transitive);
        } else {
            for watcher in all_watchers {
                if self.is_live(watcher) && !self.slots[watcher.slot].is_terminal() {
                    self.enqueue_ready(watcher);
                }
            }
        }

        rep_dst
    }

    /// Mark `key`'s producer as failed and propagate the failure to all direct
    /// and transitive observers so no dependent remains permanently blocked.
    pub(in crate::program) fn fail_producer(
        &mut self,
        key: &DependencyKey,
        detail: impl Into<String>,
    ) -> Vec<GoalRef> {
        let rep = self.find_representative(key);
        let detail = detail.into();
        let mut queue = VecDeque::from([(rep, detail)]);
        let mut visited_keys = BTreeSet::new();
        let mut affected = Vec::new();

        while let Some((failed_key, detail)) = queue.pop_front() {
            let rep = self.find_representative(&failed_key);
            if !visited_keys.insert(rep.clone()) {
                continue;
            }
            let watchers = {
                let entry = self.dependencies.entry(rep.clone()).or_default();
                entry.version = entry.version.checked_add(1).unwrap_or(u64::MAX);
                entry.status = ProducerStatus::Failed(detail.clone());
                entry.producer = None;
                std::mem::take(&mut entry.watchers)
            };
            for watcher in watchers {
                if !self.is_live(watcher) || self.slots[watcher.slot].is_terminal() {
                    continue;
                }
                let reason = FailureReason::ProducerFailed {
                    dependency: rep.clone(),
                    detail: detail.clone(),
                };
                let produced = self.fail_goal_with_reason(watcher, reason);
                affected.push(watcher);
                for (next_key, next_detail) in produced {
                    queue.push_back((next_key, next_detail));
                }
            }
        }

        affected
    }

    fn propagate_failed_keys(&mut self, initial: Vec<(DependencyKey, String)>) {
        let mut queue = VecDeque::from(initial);
        let mut visited_keys = BTreeSet::new();
        while let Some((failed_key, detail)) = queue.pop_front() {
            let rep = self.find_representative(&failed_key);
            if !visited_keys.insert(rep.clone()) {
                continue;
            }
            let watchers = {
                let entry = self.dependencies.entry(rep.clone()).or_default();
                entry.version = entry.version.checked_add(1).unwrap_or(u64::MAX);
                entry.status = ProducerStatus::Failed(detail.clone());
                entry.producer = None;
                std::mem::take(&mut entry.watchers)
            };
            for watcher in watchers {
                if !self.is_live(watcher) || self.slots[watcher.slot].is_terminal() {
                    continue;
                }
                let reason = FailureReason::ProducerFailed {
                    dependency: rep.clone(),
                    detail: detail.clone(),
                };
                queue.extend(self.fail_goal_with_reason(watcher, reason));
            }
        }
    }

    fn fail_goal_with_reason(
        &mut self,
        goal: GoalRef,
        reason: FailureReason,
    ) -> Vec<(DependencyKey, String)> {
        if !self.is_live(goal) {
            return Vec::new();
        }
        self.clear_goal_watches(goal);
        self.ready_set.remove(&goal);
        let detail = reason.to_string();
        let produced = {
            let slot = &mut self.slots[goal.slot];
            slot.state = GoalState::Failed(reason.clone());
            slot.occupied = false;
            slot.generation = slot.generation.checked_add(1).unwrap_or(u64::MAX);
            std::mem::take(&mut slot.produces)
        };
        self.free_slots.insert(goal.slot);
        self.failed_goals.insert(goal, reason);
        produced
            .into_iter()
            .map(|dep| (dep, detail.clone()))
            .collect()
    }

    /// Complete `goal`, removing all its watcher registrations and advancing its
    /// slot generation so any retained wait ticket becomes permanently stale.
    pub(in crate::program) fn complete_goal(&mut self, goal: GoalRef) -> bool {
        if !self.is_live(goal) {
            return false;
        }
        self.clear_goal_watches(goal);
        self.ready_set.remove(&goal);
        let produced = {
            let slot = &mut self.slots[goal.slot];
            slot.state = GoalState::Completed;
            slot.occupied = false;
            slot.generation = slot.generation.checked_add(1).unwrap_or(u64::MAX);
            std::mem::take(&mut slot.produces)
        };
        for key in produced {
            let rep = self.find_representative(&key);
            if let Some(entry) = self.dependencies.get_mut(&rep)
                && entry.producer == Some(goal)
            {
                entry.producer = None;
            }
        }
        self.free_slots.insert(goal.slot);
        self.completed_count += 1;
        true
    }

    /// Cancel `goal`, cleaning up its watchers, advancing its slot generation,
    /// and propagating failure to any goals waiting on keys it produced.
    pub(in crate::program) fn cancel_goal(
        &mut self,
        goal: GoalRef,
        detail: impl Into<String>,
    ) -> bool {
        if !self.is_live(goal) {
            return false;
        }
        self.clear_goal_watches(goal);
        self.ready_set.remove(&goal);
        let detail = detail.into();
        let reason = FailureReason::Cancelled {
            detail: detail.clone(),
        };
        let produced = {
            let slot = &mut self.slots[goal.slot];
            slot.state = GoalState::Cancelled(reason.clone());
            slot.occupied = false;
            slot.generation = slot.generation.checked_add(1).unwrap_or(u64::MAX);
            std::mem::take(&mut slot.produces)
        };
        self.free_slots.insert(goal.slot);
        self.failed_goals.insert(goal, reason);
        let transitive = produced
            .into_iter()
            .map(|dep| (dep, detail.clone()))
            .collect();
        self.propagate_failed_keys(transitive);
        true
    }

    fn clear_goal_watches(&mut self, goal: GoalRef) {
        let watches = std::mem::take(&mut self.slots[goal.slot].watches);
        for key in watches {
            let rep = self.find_representative(&key);
            if let Some(entry) = self.dependencies.get_mut(&rep) {
                entry.watchers.remove(&goal);
            }
        }
        if let GoalState::Waiting(keys) = &mut self.slots[goal.slot].state {
            keys.clear();
        }
    }

    /// Detect and cancel topological dependency cycles among currently live
    /// waiting goals, propagating failure to any downstream observers.
    pub(in crate::program) fn cancel_cycles(&mut self) -> Vec<Vec<GoalRef>> {
        let mut live_goals = Vec::new();
        let mut graph: BTreeMap<GoalRef, BTreeSet<GoalRef>> = BTreeMap::new();
        for (slot_idx, slot) in self.slots.iter().enumerate() {
            if !slot.occupied {
                continue;
            }
            let goal = GoalRef {
                slot: slot_idx,
                generation: slot.generation,
            };
            live_goals.push(goal);
            let mut edges = BTreeSet::new();
            for dep in &slot.watches {
                let rep = self.peek_representative(dep);
                if let Some(entry) = self.dependencies.get(&rep)
                    && let Some(producer) = entry.producer
                    && self.is_live(producer)
                {
                    edges.insert(producer);
                }
            }
            graph.insert(goal, edges);
        }

        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Color {
            Unvisited,
            Visiting,
            Visited,
        }

        let mut color: BTreeMap<GoalRef, Color> = live_goals
            .iter()
            .map(|&goal| (goal, Color::Unvisited))
            .collect();
        let mut stack: Vec<GoalRef> = Vec::new();
        let mut cycles: Vec<Vec<GoalRef>> = Vec::new();
        let mut in_cycle: BTreeSet<GoalRef> = BTreeSet::new();

        fn dfs(
            node: GoalRef,
            graph: &BTreeMap<GoalRef, BTreeSet<GoalRef>>,
            color: &mut BTreeMap<GoalRef, Color>,
            stack: &mut Vec<GoalRef>,
            cycles: &mut Vec<Vec<GoalRef>>,
            in_cycle: &mut BTreeSet<GoalRef>,
        ) {
            color.insert(node, Color::Visiting);
            stack.push(node);
            if let Some(neighbors) = graph.get(&node) {
                for &next in neighbors {
                    match color.get(&next).copied().unwrap_or(Color::Visited) {
                        Color::Unvisited => dfs(next, graph, color, stack, cycles, in_cycle),
                        Color::Visiting => {
                            if let Some(pos) = stack.iter().position(|&g| g == next) {
                                let cycle = stack[pos..].to_vec();
                                for &member in &cycle {
                                    in_cycle.insert(member);
                                }
                                cycles.push(cycle);
                            }
                        }
                        Color::Visited => {}
                    }
                }
            }
            stack.pop();
            color.insert(node, Color::Visited);
        }

        for goal in live_goals {
            if color.get(&goal) == Some(&Color::Unvisited) {
                dfs(
                    goal,
                    &graph,
                    &mut color,
                    &mut stack,
                    &mut cycles,
                    &mut in_cycle,
                );
            }
        }

        if cycles.is_empty() {
            return cycles;
        }

        let mut transitive = Vec::new();
        for cycle in &cycles {
            let reason = FailureReason::Cycle {
                path: cycle.clone(),
            };
            for &goal in cycle {
                if self.is_live(goal) {
                    transitive.extend(self.fail_goal_with_reason(goal, reason.clone()));
                }
            }
        }
        self.propagate_failed_keys(transitive);
        cycles
    }

    /// Capture a rollback checkpoint of the registry state.
    pub(in crate::program) fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            slots: self.slots.clone(),
            free_slots: self.free_slots.clone(),
            parent: self.parent.clone(),
            dependencies: self.dependencies.clone(),
            ready_queue: self.ready_queue.clone(),
            ready_set: self.ready_set.clone(),
            completed_count: self.completed_count,
            failed_goals: self.failed_goals.clone(),
        }
    }

    /// Restore the registry to `checkpoint`, cleanly discarding speculative
    /// goals and watchers while strictly advancing generations on every slot
    /// touched or allocated since `checkpoint` so stale tickets never wake
    /// future occupants of the same slot index.
    pub(in crate::program) fn rollback(&mut self, checkpoint: Checkpoint) {
        let mut preserved_generations = Vec::with_capacity(self.slots.len());
        for (idx, current) in self.slots.iter().enumerate() {
            let saved = checkpoint.slots.get(idx);
            let bumped = match saved {
                Some(prev) if prev == current => current.generation,
                Some(prev) => current
                    .generation
                    .max(prev.generation)
                    .checked_add(1)
                    .unwrap_or(u64::MAX),
                None => current.generation.checked_add(1).unwrap_or(u64::MAX),
            };
            preserved_generations.push(bumped);
        }

        self.slots = checkpoint.slots;
        self.free_slots = checkpoint.free_slots;
        self.parent = checkpoint.parent;
        self.dependencies = checkpoint.dependencies;
        self.ready_queue = checkpoint.ready_queue;
        self.ready_set = checkpoint.ready_set;
        self.completed_count = checkpoint.completed_count;
        self.failed_goals = checkpoint.failed_goals;

        // Keep slot length >= preserved_generations so retired speculative slots
        // retain their bumped generation counter across future allocations.
        for (idx, generation) in preserved_generations.into_iter().enumerate() {
            if let Some(slot) = self.slots.get_mut(idx) {
                if slot.generation != generation {
                    let old_ref = GoalRef {
                        slot: idx,
                        generation: slot.generation,
                    };
                    slot.generation = generation;
                    if slot.occupied {
                        let new_ref = GoalRef {
                            slot: idx,
                            generation,
                        };
                        if self.ready_set.remove(&old_ref) {
                            self.ready_set.insert(new_ref);
                            for item in &mut self.ready_queue {
                                if *item == old_ref {
                                    *item = new_ref;
                                }
                            }
                        }
                        for dep in &slot.watches {
                            let rep = Self::peek_representative_in(&self.parent, dep);
                            if let Some(entry) = self.dependencies.get_mut(&rep) {
                                if entry.watchers.remove(&old_ref) {
                                    entry.watchers.insert(new_ref);
                                }
                            }
                        }
                        for dep in &slot.produces {
                            let rep = Self::peek_representative_in(&self.parent, dep);
                            if let Some(entry) = self.dependencies.get_mut(&rep)
                                && entry.producer == Some(old_ref)
                            {
                                entry.producer = Some(new_ref);
                            }
                        }
                    }
                }
            } else {
                self.free_slots.insert(idx);
                self.slots.push(SlotRecord {
                    generation,
                    occupied: false,
                    state: GoalState::Completed,
                    watches: BTreeSet::new(),
                    produces: BTreeSet::new(),
                    label: None,
                });
            }
        }
    }

    /// Finish a local region per `local-dependencies.md`:
    /// detect and cancel any cycles, propagate failed producers, reject
    /// remaining unresolved waits, and clean up all local tasks/watchers.
    pub(in crate::program) fn finish(&mut self) -> FinishReport {
        let mut report = FinishReport {
            completed: self.completed_count,
            runnable_remaining: self.ready_set.len(),
            ..FinishReport::default()
        };

        let cycles = self.cancel_cycles();
        report.cycles_cancelled = cycles;

        let waiting_goals: Vec<(GoalRef, Option<DependencyKey>)> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(slot_idx, slot)| {
                if !slot.occupied {
                    return None;
                }
                let goal = GoalRef {
                    slot: slot_idx,
                    generation: slot.generation,
                };
                match &slot.state {
                    GoalState::Waiting(deps) => Some((goal, deps.first().cloned())),
                    GoalState::Runnable => Some((goal, None)),
                    _ => None,
                }
            })
            .collect();

        for (goal, dep) in waiting_goals {
            if !self.is_live(goal) {
                continue;
            }
            if let Some(dep) = dep {
                let rep = self.find_representative(&dep);
                let failed_detail =
                    self.dependencies
                        .get(&rep)
                        .and_then(|entry| match &entry.status {
                            ProducerStatus::Failed(detail) => Some(detail.clone()),
                            _ => None,
                        });
                let reason = if let Some(detail) = failed_detail {
                    FailureReason::ProducerFailed {
                        dependency: rep,
                        detail,
                    }
                } else {
                    report.unresolved_cancelled.push((goal, rep.clone()));
                    FailureReason::Unresolved { dependency: rep }
                };
                let transitive = self.fail_goal_with_reason(goal, reason);
                self.propagate_failed_keys(transitive);
            } else {
                self.cancel_goal(goal, "unfinished runnable goal at region finish");
            }
        }

        report.failed = self
            .failed_goals
            .iter()
            .map(|(goal, reason)| (*goal, reason.clone()))
            .collect();
        self.ready_queue.clear();
        self.ready_set.clear();
        report
    }
}

impl std::fmt::Display for WatcherRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(watcher-registry (goals")?;
        for (idx, slot) in self.slots.iter().enumerate() {
            if slot.occupied {
                write!(f, " {}", slot.as_goal(idx))?;
            }
        }
        f.write_str(") (ready")?;
        for goal in &self.ready_queue {
            write!(f, " {goal}")?;
        }
        f.write_str(") (dependencies")?;
        for (key, entry) in &self.dependencies {
            write!(
                f,
                " (dep {key} (version {}) (status {}) (watchers",
                entry.version, entry.status
            )?;
            for watcher in &entry.watchers {
                write!(f, " {watcher}")?;
            }
            f.write_str("))")?;
        }
        f.write_str(") (aliases")?;
        for (from, to) in &self.parent {
            write!(f, " ({from} -> {to})")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_checked_registration_enqueues_on_interleaved_update() {
        let mut reg = WatcherRegistry::new();
        let goal = reg.spawn_waiting_goal(Some("lambda".into()));
        let key = DependencyKey::Slot(1);

        let (rep, observed_version) = reg.observe_version(&key);
        assert_eq!(observed_version, 0);

        // Interleaved update occurs between observe and register.
        reg.publish_update(&key);

        let ticket = reg.register_watch(goal, rep, observed_version).unwrap();
        assert_eq!(ticket.observed_version, 0);
        assert_eq!(reg.ready_len(), 1);
        assert_eq!(reg.pop_ready(), Some(goal));
    }

    #[test]
    fn generation_keyed_deduplication_coalesces_multiple_dependency_wakes() {
        let mut reg = WatcherRegistry::new();
        let goal = reg.spawn_waiting_goal(Some("multi-waiter".into()));

        for slot in [0, 1, 2] {
            let (rep, ver) = reg.observe_version(&DependencyKey::Slot(slot));
            reg.register_watch(goal, rep, ver).unwrap();
        }

        // All three dependencies fire before the goal runs.
        assert_eq!(reg.publish_update(&DependencyKey::Slot(0)), 1);
        assert_eq!(reg.publish_update(&DependencyKey::Slot(1)), 0);
        assert_eq!(reg.publish_update(&DependencyKey::Slot(2)), 0);

        assert_eq!(reg.ready_len(), 1);
        assert_eq!(reg.pop_ready(), Some(goal));
        assert_eq!(reg.pop_ready(), None);

        // Popping cleared prior watches; re-publishing no longer wakes `goal`.
        assert_eq!(reg.publish_update(&DependencyKey::Slot(0)), 0);
    }

    #[test]
    fn stale_generation_ticket_never_wakes_reused_slot() {
        let mut reg = WatcherRegistry::new();
        let g1 = reg.spawn_waiting_goal(Some("first".into()));
        let (rep_a, ver_a) = reg.observe_version(&DependencyKey::Tag(10));
        let stale_ticket = reg.register_watch(g1, rep_a, ver_a).unwrap();
        assert_eq!(
            g1,
            GoalRef {
                slot: 0,
                generation: 0
            }
        );

        assert!(reg.complete_goal(g1));

        // Slot 0 is reused with generation 1.
        let g2 = reg.spawn_waiting_goal(Some("second".into()));
        assert_eq!(
            g2,
            GoalRef {
                slot: 0,
                generation: 1
            }
        );
        let (rep_b, ver_b) = reg.observe_version(&DependencyKey::Tag(20));
        let live_ticket = reg.register_watch(g2, rep_b, ver_b).unwrap();

        // Stale ticket and old dependency update must not wake `g2`.
        assert!(!reg.wake_ticket(&stale_ticket));
        assert_eq!(reg.publish_update(&DependencyKey::Tag(10)), 0);
        assert_eq!(reg.pop_ready(), None);

        // Live ticket wakes `g2`.
        assert!(reg.wake_ticket(&live_ticket));
        assert_eq!(reg.pop_ready(), Some(g2));
    }

    #[test]
    fn union_find_merge_forwards_watchers_and_wakes_original_observers() {
        let mut reg = WatcherRegistry::new();
        let g_left = reg.spawn_waiting_goal(Some("left-observer".into()));
        let g_right = reg.spawn_waiting_goal(Some("right-observer".into()));

        let (k1, v1) = reg.observe_version(&DependencyKey::Tag(1));
        let (k2, v2) = reg.observe_version(&DependencyKey::Tag(2));
        reg.register_watch(g_left, k1, v1).unwrap();
        reg.register_watch(g_right, k2, v2).unwrap();

        // Merging Tag(1) -> Tag(2) forwards `g_left` onto Tag(2) and wakes both.
        let rep = reg.merge_dependencies(DependencyKey::Tag(1), DependencyKey::Tag(2));
        assert_eq!(rep, DependencyKey::Tag(2));
        assert_eq!(reg.pop_ready(), Some(g_left));
        assert_eq!(reg.pop_ready(), Some(g_right));
        assert_eq!(reg.pop_ready(), None);

        // Re-observing through Tag(1) now attaches to representative Tag(2).
        let (rep_1, ver_1) = reg.observe_version(&DependencyKey::Tag(1));
        assert_eq!(rep_1, DependencyKey::Tag(2));
        let (rep_2, ver_2) = reg.observe_version(&DependencyKey::Tag(2));
        assert_eq!(rep_2, DependencyKey::Tag(2));
        reg.register_watch(g_left, rep_1, ver_1).unwrap();
        reg.register_watch(g_right, rep_2, ver_2).unwrap();

        // Publishing on the old alias Tag(1) wakes BOTH original observers.
        assert_eq!(reg.publish_update(&DependencyKey::Tag(1)), 2);
        assert_eq!(reg.pop_ready(), Some(g_left));
        assert_eq!(reg.pop_ready(), Some(g_right));
    }

    #[test]
    fn producer_failure_cancels_dependents_without_leaving_permanent_waits() {
        let mut reg = WatcherRegistry::new();
        let g_mid = reg.spawn_waiting_goal(Some("mid".into()));
        let g_leaf = reg.spawn_waiting_goal(Some("leaf".into()));

        reg.bind_producer(DependencyKey::Slot(2), g_mid);
        let (k1, v1) = reg.observe_version(&DependencyKey::Slot(1));
        reg.register_watch(g_mid, k1, v1).unwrap();
        let (k2, v2) = reg.observe_version(&DependencyKey::Slot(2));
        reg.register_watch(g_leaf, k2, v2).unwrap();

        let affected = reg.fail_producer(&DependencyKey::Slot(1), "unknown_value not bound");
        assert_eq!(affected, vec![g_mid, g_leaf]);
        assert!(!reg.is_live(g_mid));
        assert!(!reg.is_live(g_leaf));
        assert!(matches!(
            reg.goal_failure(g_mid),
            Some(FailureReason::ProducerFailed { .. })
        ));
        assert!(matches!(
            reg.goal_failure(g_leaf),
            Some(FailureReason::ProducerFailed { .. })
        ));

        // Subsequent watch on an already-failed producer fails immediately.
        let g_late = reg.spawn_waiting_goal(Some("late".into()));
        let (k1_late, v1_late) = reg.observe_version(&DependencyKey::Slot(1));
        let err = reg.register_watch(g_late, k1_late, v1_late).unwrap_err();
        assert!(matches!(err, FailureReason::ProducerFailed { .. }));

        let report = reg.finish();
        assert!(report.unresolved_cancelled.is_empty(), "{report}");
        assert_eq!(report.failed.len(), 3);
    }

    #[test]
    fn topological_cycle_detection_cancels_cycles_and_downstream_observers() {
        let mut reg = WatcherRegistry::new();
        let g0 = reg.spawn_waiting_goal(Some("slot-0".into()));
        let g1 = reg.spawn_waiting_goal(Some("slot-1".into()));
        let g2 = reg.spawn_waiting_goal(Some("slot-2".into()));
        let g_observer = reg.spawn_waiting_goal(Some("observer".into()));

        reg.bind_producer(DependencyKey::Slot(0), g0);
        reg.bind_producer(DependencyKey::Slot(1), g1);
        reg.bind_producer(DependencyKey::Slot(2), g2);

        let (k1, v1) = reg.observe_version(&DependencyKey::Slot(1));
        reg.register_watch(g0, k1, v1).unwrap();
        let (k2, v2) = reg.observe_version(&DependencyKey::Slot(2));
        reg.register_watch(g1, k2, v2).unwrap();
        let (k0, v0) = reg.observe_version(&DependencyKey::Slot(0));
        reg.register_watch(g2, k0, v0).unwrap();

        let (k1_obs, v1_obs) = reg.observe_version(&DependencyKey::Slot(1));
        reg.register_watch(g_observer, k1_obs, v1_obs).unwrap();

        let report = reg.finish();
        assert_eq!(report.cycles_cancelled, vec![vec![g0, g1, g2]]);
        assert!(report.unresolved_cancelled.is_empty(), "{report}");
        assert!(matches!(
            reg.goal_failure(g0),
            Some(FailureReason::Cycle { .. })
        ));
        assert!(matches!(
            reg.goal_failure(g_observer),
            Some(FailureReason::ProducerFailed { .. })
        ));
        assert!(!reg.is_live(g0));
        assert!(!reg.is_live(g1));
        assert!(!reg.is_live(g2));
        assert!(!reg.is_live(g_observer));
    }

    #[test]
    fn rollback_cleans_speculative_watchers_and_invalidates_discarded_tickets() {
        let mut reg = WatcherRegistry::new();
        let outer = reg.spawn_waiting_goal(Some("outer".into()));
        let (k_outer, v_outer) = reg.observe_version(&DependencyKey::Tag(1));
        reg.register_watch(outer, k_outer, v_outer).unwrap();

        let cp = reg.checkpoint();

        let probe = reg.spawn_waiting_goal(Some("probe".into()));
        let (k_probe, v_probe) = reg.observe_version(&DependencyKey::Tag(2));
        let probe_ticket = reg.register_watch(probe, k_probe, v_probe).unwrap();
        assert_eq!(
            probe,
            GoalRef {
                slot: 1,
                generation: 0
            }
        );

        reg.rollback(cp);

        // Speculative watcher is gone and its ticket cannot wake a reused slot.
        assert_eq!(reg.publish_update(&DependencyKey::Tag(2)), 0);
        let replacement = reg.spawn_waiting_goal(Some("replacement".into()));
        assert_eq!(replacement.slot, 1);
        assert!(replacement.generation > probe_ticket.goal.generation);
        assert!(!reg.wake_ticket(&probe_ticket));

        // Pre-checkpoint goal remains intact and wakes cleanly.
        assert_eq!(reg.publish_update(&DependencyKey::Tag(1)), 1);
        assert_eq!(reg.pop_ready(), Some(outer));
    }

    #[test]
    fn deterministic_replay_and_sexpr_escaping() {
        let run = || {
            let mut reg = WatcherRegistry::new();
            assert!(reg.is_empty());
            let a = reg.spawn_waiting_goal(Some("goal \"a\"\n1".into()));
            let b = reg.spawn_waiting_goal(Some("goal \"b\"\n2".into()));
            assert_eq!(reg.goal(a).unwrap().label.as_deref(), Some("goal \"a\"\n1"));
            let (k1, v1) =
                reg.observe_version(&DependencyKey::Local(n::LocalId { owner: 1, index: 2 }));
            let (k2, v2) =
                reg.observe_version(&DependencyKey::Local(n::LocalId { owner: 1, index: 3 }));
            reg.register_watch(a, k1, v1).unwrap();
            reg.register_watch(b, k2, v2).unwrap();
            reg.merge_dependencies(
                DependencyKey::Local(n::LocalId { owner: 1, index: 2 }),
                DependencyKey::Local(n::LocalId { owner: 1, index: 3 }),
            );
            let dump = reg.to_string();
            let popped = std::iter::from_fn(|| reg.pop_ready()).collect::<Vec<_>>();
            // Re-register under the merged representative before producer failure.
            let (rep_a, ver_a) =
                reg.observe_version(&DependencyKey::Local(n::LocalId { owner: 1, index: 2 }));
            let (rep_b, ver_b) =
                reg.observe_version(&DependencyKey::Local(n::LocalId { owner: 1, index: 3 }));
            reg.register_watch(a, rep_a, ver_a).unwrap();
            reg.register_watch(b, rep_b, ver_b).unwrap();
            let failed = reg.fail_producer(
                &DependencyKey::Local(n::LocalId { owner: 1, index: 3 }),
                "detail \"x\"\nline2",
            );
            assert_eq!(failed, vec![a, b]);
            let report = reg.finish();
            assert!(!report.is_clean());
            (dump, popped, report.to_string())
        };

        let (d1, p1, r1) = run();
        let (d2, p2, r2) = run();
        assert_eq!(d1, d2);
        assert_eq!(p1, p2);
        assert_eq!(r1, r2);
        assert!(d1.starts_with("(watcher-registry "), "{d1}");
        assert!(d1.contains(r#"goal \"a\"\n1"#), "{d1}");
        assert!(r1.contains(r#"detail \"x\"\nline2"#), "{r1}");
    }
}
