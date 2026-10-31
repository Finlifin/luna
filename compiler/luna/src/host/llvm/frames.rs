//! Suspension of local resource storage and its exact published scope.
//! LLVM SSA names, instructions, receipt-path IDs, and return/resume targets
//! belong to the enclosing machine function and are deliberately not reset.
use super::*;

#[derive(Clone)]
pub(super) struct LoopTarget {
    pub(super) control: Option<(String, String)>,
    pub(super) head: String,
    pub(super) end: String,
    pub(super) owner: Option<(iir::InstanceKey, Vec<iir::cleanup::ContextKey>)>,
}
impl LoopTarget {
    pub(super) fn belongs_to(&self, scope: Option<&iir::cleanup::FunctionSchedule>) -> bool {
        match (&self.owner, scope) {
            (None, None) => true,
            (Some((instance, context)), Some(scope)) => {
                instance == &scope.instance && context == &scope.context
            }
            _ => false,
        }
    }
}

#[derive(Clone)]
pub(super) struct LocalFrame<'a> {
    cleanup: Option<&'a iir::cleanup::FunctionSchedule>,
    pub(super) locals: BTreeMap<LocalId, Slot>,
    live: BTreeMap<(LocalId, Vec<usize>), Slot>,
    temporaries: BTreeMap<(u32, u32), Slot>,
    temporary_live: BTreeMap<(u32, u32), Slot>,
    prepared: BTreeMap<(u32, u32, usize), cleanup::PreparedSlot>,
    pattern_orders: BTreeMap<(u32, u32), Slot>,
    temporary_fields: BTreeMap<((u32, u32), Vec<usize>), Slot>,
    loops: BTreeMap<LocalId, frames::LoopTarget>,
}
/// Suspended resource scopes in actual evaluation order, including defaults
/// and effect requesters. One stack preserves interleaved failure cleanup order.
#[derive(Clone)]
pub(super) struct UnwindFrame<'a> {
    pub(super) state: LocalFrame<'a>,
    pub(super) source: nir::program::SourceRange,
    pub(super) boundary: iir::cleanup::Boundary,
}

impl<'a> Emitter<'a> {
    /// Return crosses handler requesters until it reaches the owning machine
    /// function's resource scope. Check its result only after that restoration.
    pub(super) fn return_from_handler(
        &mut self,
        value: &Value,
        source: nir::program::SourceRange,
    ) -> Result<(), String> {
        use iir::cleanup::{Boundary, ContextKey};
        *self.fuel = self
            .fuel
            .checked_sub(self.resource_unwinds.len())
            .ok_or("host handler return budget exhausted")?;
        let destination = self
            .resource_unwinds
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, frame)| {
                let scope = frame.state.cleanup?;
                (!matches!(scope.context.last(), Some(ContextKey::EffectHandler { .. })))
                    .then_some((index, scope))
            })
            .ok_or("host handler return destination missing")?;
        if self.resource_unwinds[destination.0..]
            .iter()
            .any(|frame| frame.boundary != Boundary::PredicateFailure)
            || matches!(
                destination.1.context.last(),
                Some(ContextKey::Default { .. })
            )
        {
            return Err("host handler return across default evaluation is not implemented".into());
        }
        self.cleanup_boundary(source, Boundary::TemporaryEnd)?;
        self.cleanup_boundary(source, Boundary::Return)?;
        let current = self.take_local_frame();
        let frames = self.resource_unwinds.clone();
        let result = (|| {
            for index in (destination.0..frames.len()).rev() {
                let frame = &frames[index];
                self.restore_local_frame(frame.state.clone());
                // Failure in a result postcondition must not unwind scopes
                // whose cleanup has already been emitted on this return path.
                self.resource_unwinds.truncate(index);
                if index == destination.0 {
                    return self.emit_return(value, Some(frame.source), Boundary::EffectReturn);
                }
                self.cleanup_boundary(frame.source, Boundary::EffectEscape)?;
            }
            unreachable!()
        })();
        self.resource_unwinds = frames;
        self.restore_local_frame(current);
        result
    }

    pub(super) fn loop_from_handler(
        &mut self,
        identity: LocalId,
        target: &LoopTarget,
        source: nir::program::SourceRange,
        is_break: bool,
    ) -> Result<(), String> {
        use iir::cleanup::Boundary;
        *self.fuel = self
            .fuel
            .checked_sub(self.resource_unwinds.len())
            .ok_or("host handler loop transfer budget exhausted")?;
        let destination = self
            .resource_unwinds
            .iter()
            .rposition(|frame| {
                target.belongs_to(frame.state.cleanup)
                    && frame.state.loops.get(&identity).is_some_and(|candidate| {
                        candidate.head == target.head && candidate.end == target.end
                    })
            })
            .ok_or("host handler lexical loop resource scope missing")?;
        if self.resource_unwinds[destination..]
            .iter()
            .any(|frame| frame.boundary != Boundary::PredicateFailure)
        {
            return Err(
                "host handler loop transfer across default evaluation is not implemented".into(),
            );
        }
        self.cleanup_boundary(
            source,
            if is_break {
                Boundary::Break
            } else {
                Boundary::Continue
            },
        )?;
        let current = self.take_local_frame();
        let frames = self.resource_unwinds.clone();
        let outcome = (|| {
            for index in (destination..frames.len()).rev() {
                let frame = &frames[index];
                self.restore_local_frame(frame.state.clone());
                self.resource_unwinds.truncate(index);
                let boundary = if index != destination {
                    Boundary::EffectEscape
                } else if is_break {
                    Boundary::EffectBreak(identity)
                } else {
                    Boundary::EffectContinue(identity)
                };
                self.cleanup_boundary(frame.source, boundary)?;
            }
            self.branch(if is_break { &target.end } else { &target.head });
            Ok(())
        })();
        self.resource_unwinds = frames;
        self.restore_local_frame(current);
        outcome
    }

    pub(super) fn unwind_resources(&mut self) -> Result<(), String> {
        if self.resource_unwinds.is_empty() {
            return Ok(());
        }
        let current = self.take_local_frame();
        let frames = self.resource_unwinds.clone();
        let result = (|| {
            for frame in frames.into_iter().rev() {
                self.restore_local_frame(frame.state);
                self.cleanup_boundary(frame.source, frame.boundary)?;
            }
            Ok(())
        })();
        self.restore_local_frame(current);
        result
    }

    /// Select before mutating emitter state. A missing/ambiguous child or an
    /// exhausted lookup budget leaves the suspended scope and storage intact.
    pub(super) fn enter_local_scope(
        &mut self,
        key: iir::cleanup::ContextKey,
    ) -> Result<LocalFrame<'a>, String> {
        let parent = self
            .cleanup
            .ok_or("host local resource scope has no parent")?;
        self.enter_local_scope_at(&parent.instance, &parent.context, key)
    }

    pub(super) fn enter_local_scope_at(
        &mut self,
        instance: &iir::InstanceKey,
        parent_context: &[iir::cleanup::ContextKey],
        key: iir::cleanup::ContextKey,
    ) -> Result<LocalFrame<'a>, String> {
        let schedule = self
            .schedules
            .ok_or("host local resource schedule missing")?;
        *self.fuel = self
            .fuel
            .checked_sub(parent_context.len().saturating_add(1))
            .ok_or("host local resource context budget exhausted")?;
        let mut context = parent_context.to_vec();
        context.push(key);
        *self.fuel = self
            .fuel
            .checked_sub(
                schedule
                    .functions
                    .len()
                    .saturating_mul(context.len().saturating_add(1)),
            )
            .ok_or("host local resource lookup budget exhausted")?;
        let child = schedule
            .exact_function(instance, &context)
            .map_err(|error| format!("host local resource context: {error}"))?;
        let frame = self.take_local_frame();
        self.cleanup = Some(child);
        Ok(frame)
    }

    pub(super) fn take_local_frame(&mut self) -> LocalFrame<'a> {
        LocalFrame {
            cleanup: self.cleanup.take(),
            locals: std::mem::take(&mut self.locals),
            live: std::mem::take(&mut self.live),
            temporaries: std::mem::take(&mut self.temporaries),
            temporary_live: std::mem::take(&mut self.temporary_live),
            prepared: std::mem::take(&mut self.prepared),
            pattern_orders: std::mem::take(&mut self.pattern_orders),
            temporary_fields: std::mem::take(&mut self.temporary_fields),
            loops: std::mem::take(&mut self.loops),
        }
    }
    pub(super) fn restore_local_frame(&mut self, state: LocalFrame<'a>) {
        self.cleanup = state.cleanup;
        self.locals = state.locals;
        self.live = state.live;
        self.temporaries = state.temporaries;
        self.temporary_live = state.temporary_live;
        self.prepared = state.prepared;
        self.pattern_orders = state.pattern_orders;
        self.temporary_fields = state.temporary_fields;
        self.loops = state.loops;
    }
}

impl Emitter<'_> {
    /// Unwind suspended local scopes on a machine effect escape. Default
    /// preparation retains its dedicated boundary; requester frames use the
    /// published effect-escape program rather than predicate-failure cleanup.
    pub(super) fn unwind_effect_frames(&mut self) -> Result<(), String> {
        use iir::cleanup::Boundary;
        *self.fuel = self
            .fuel
            .checked_sub(self.resource_unwinds.len())
            .ok_or("host effect unwind budget exhausted")?;
        let current = self.take_local_frame();
        let frames = self.resource_unwinds.clone();
        let outcome = (|| {
            for frame in frames.into_iter().rev() {
                self.restore_local_frame(frame.state);
                self.cleanup_boundary(
                    frame.source,
                    if frame.boundary == Boundary::PredicateFailure {
                        Boundary::EffectEscape
                    } else {
                        frame.boundary
                    },
                )?;
            }
            Ok(())
        })();
        self.restore_local_frame(current);
        outcome
    }

    /// Re-enter the installer's lexical function frame before checking its
    /// return contract. Intervening handler scopes must be cleaned first.
    pub(super) fn receive_effect_return(
        &mut self,
        value: &Value,
        source: nir::program::SourceRange,
    ) -> Result<(), String> {
        use iir::cleanup::{Boundary, ContextKey};
        *self.fuel = self
            .fuel
            .checked_sub(self.resource_unwinds.len())
            .ok_or("host effect return receipt budget exhausted")?;
        let in_child = self.cleanup.is_some_and(|scope| {
            matches!(
                scope.context.last(),
                Some(ContextKey::EffectHandler { .. } | ContextKey::Default { .. })
            )
        });
        if !in_child {
            return self.emit_return(value, Some(source), Boundary::EffectReturn);
        }
        if self
            .cleanup
            .is_some_and(|scope| matches!(scope.context.last(), Some(ContextKey::Default { .. })))
            || self
                .resource_unwinds
                .iter()
                .any(|frame| frame.boundary != Boundary::PredicateFailure)
        {
            return Err("host effect return across default evaluation is not implemented".into());
        }
        self.cleanup_boundary(source, Boundary::EffectEscape)?;
        let current = self.take_local_frame();
        let frames = self.resource_unwinds.clone();
        let outcome = (|| {
            for index in (0..frames.len()).rev() {
                let frame = &frames[index];
                self.restore_local_frame(frame.state.clone());
                self.resource_unwinds.truncate(index);
                let in_child = self.cleanup.is_some_and(|scope| {
                    matches!(
                        scope.context.last(),
                        Some(ContextKey::EffectHandler { .. } | ContextKey::Default { .. })
                    )
                });
                if !in_child {
                    return self.emit_return(value, Some(frame.source), Boundary::EffectReturn);
                }
                self.cleanup_boundary(
                    frame.source,
                    if frame.boundary == Boundary::PredicateFailure {
                        Boundary::EffectEscape
                    } else {
                        frame.boundary
                    },
                )?;
            }
            Err("host effect return installer frame is missing".into())
        })();
        self.resource_unwinds = frames;
        self.restore_local_frame(current);
        outcome
    }
}

impl Emitter<'_> {
    pub(super) fn receive_effect_loop(
        &mut self,
        identity: LocalId,
        target: &LoopTarget,
        source: nir::program::SourceRange,
        is_break: bool,
    ) -> Result<(), String> {
        use iir::cleanup::Boundary;
        let boundary = if is_break {
            Boundary::EffectBreak(identity)
        } else {
            Boundary::EffectContinue(identity)
        };
        if target.belongs_to(self.cleanup) {
            self.cleanup_boundary(source, boundary)?;
            self.branch(if is_break { &target.end } else { &target.head });
            return Ok(());
        }
        if self
            .resource_unwinds
            .iter()
            .any(|frame| frame.boundary != Boundary::PredicateFailure)
        {
            return Err(
                "host effect loop transfer across default evaluation is not implemented".into(),
            );
        }
        *self.fuel = self
            .fuel
            .checked_sub(self.resource_unwinds.len())
            .ok_or("host effect loop receipt budget exhausted")?;
        self.cleanup_boundary(source, Boundary::EffectEscape)?;
        let current = self.take_local_frame();
        let frames = self.resource_unwinds.clone();
        let outcome = (|| {
            for (index, frame) in frames.iter().enumerate().rev() {
                self.restore_local_frame(frame.state.clone());
                self.resource_unwinds.truncate(index);
                if target.belongs_to(self.cleanup) {
                    self.cleanup_boundary(frame.source, boundary)?;
                    self.branch(if is_break { &target.end } else { &target.head });
                    return Ok(());
                }
                self.cleanup_boundary(frame.source, Boundary::EffectEscape)?;
            }
            Err("host effect loop installer frame is missing".into())
        })();
        self.resource_unwinds = frames;
        self.restore_local_frame(current);
        outcome
    }
}
