//! Control outcomes remain distinct until their lexical target joins them.
//! A return path never contributes a prefix to later statements in its block.
use super::*;
use nir::program::LocalId;
use std::collections::BTreeMap;
mod matching;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Exit {
    Return,
    Break(LocalId),
    Continue(LocalId),
    Resume,
    Abort,
}
#[derive(Default)]
pub(super) struct Paths {
    normal: Option<Uses>,
    exits: BTreeMap<Exit, Uses>,
}
impl Paths {
    fn normal(uses: Uses) -> Self {
        Self {
            normal: Some(uses),
            exits: BTreeMap::new(),
        }
    }
    fn exit(&mut self, target: Exit, uses: Uses, fuel: &mut usize) -> Result<(), String> {
        step(fuel, 1)?;
        if let Some(previous) = self.exits.get_mut(&target) {
            join(previous, &uses, fuel)?;
        } else {
            self.exits.insert(target, uses);
        }
        Ok(())
    }
    fn divert(&mut self, target: Exit, fuel: &mut usize) -> Result<(), String> {
        if let Some(uses) = self.normal.take() {
            self.exit(target, uses, fuel)?;
        }
        Ok(())
    }
    fn merge(&mut self, other: Self, fuel: &mut usize) -> Result<(), String> {
        if let Some(uses) = other.normal {
            if let Some(normal) = &mut self.normal {
                join(normal, &uses, fuel)?;
            } else {
                self.normal = Some(uses);
            }
        }
        for (target, uses) in other.exits {
            self.exit(target, uses, fuel)?;
        }
        Ok(())
    }
    fn then(&mut self, mut next: Self, fuel: &mut usize) -> Result<(), String> {
        let Some(prefix) = self.normal.take() else {
            return Ok(());
        };
        if let Some(normal) = &mut next.normal {
            add(normal, &prefix, 1, fuel)?;
        }
        for uses in next.exits.values_mut() {
            add(uses, &prefix, 1, fuel)?;
        }
        self.merge(next, fuel)
    }
    pub(super) fn upper(self, n: usize, fuel: &mut usize) -> Result<Uses, String> {
        step(fuel, n + 1)?;
        let mut upper = self.normal.unwrap_or_else(|| vec![0; n]);
        for uses in self.exits.values() {
            join(&mut upper, uses, fuel)?;
        }
        Ok(upper)
    }
}
impl Analyzer<'_, '_, '_, '_> {
    fn sequence<'e>(
        &mut self,
        children: impl IntoIterator<Item = &'e Expr>,
        n: usize,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Paths, String> {
        step(fuel, n + 1)?;
        let mut paths = Paths::normal(vec![0; n]);
        for child in children {
            if paths.normal.is_none() {
                break;
            }
            paths.then(self.control(child, n, fuel, depth + 1)?, fuel)?;
        }
        Ok(paths)
    }
    /// Unsupported control correlations retain all syntactic exits as well as
    /// normal completion. In particular a hidden break must not turn a loop
    /// into a proven infinite loop and erase its reachable successor.
    fn conservative(&mut self, expr: &Expr, uses: Uses, fuel: &mut usize) -> Result<Paths, String> {
        let mut paths = Paths::normal(uses.clone());
        iir::visit::expression(expr, fuel, |child, fuel| {
            let target = match child.kind {
                K::Return(_) => Some(Exit::Return),
                K::Break(id) => Some(Exit::Break(id)),
                K::Continue(id) => Some(Exit::Continue(id)),
                K::EffectResume(_) => Some(Exit::Resume),
                _ => None,
            };
            if let Some(target) = target {
                step(fuel, uses.len())?;
                paths.exit(target, uses.clone(), fuel)?;
            }
            Ok(())
        })?;
        if expr.ty == Term::Primitive(nir::Primitive::Never) {
            paths.divert(Exit::Abort, fuel)?;
        }
        Ok(paths)
    }
    pub(super) fn control(
        &mut self,
        expr: &Expr,
        n: usize,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Paths, String> {
        step(fuel, n + 1)?;
        if depth >= 128 {
            return Err("effect usage control depth exhausted".into());
        }
        let mut paths = match &expr.kind {
            K::EffectHandle {
                computation,
                handlers,
                ..
            } => self.handler_installation(expr, computation, handlers, n, fuel, depth + 1),
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                self.sequence(statements.iter().chain(result.as_deref()), n, fuel, depth)
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let mut prefix = self.control(condition, n, fuel, depth + 1)?;
                if prefix.normal.is_none() {
                    return Ok(prefix);
                }
                let mut branches = Paths::default();
                let known = completed_bool(condition, fuel, 0)?;
                if known != Some(false) {
                    branches.merge(self.control(then_branch, n, fuel, depth + 1)?, fuel)?;
                }
                if known != Some(true) {
                    let branch = match else_branch {
                        Some(branch) => self.control(branch, n, fuel, depth + 1)?,
                        None => Paths::normal(vec![0; n]),
                    };
                    branches.merge(branch, fuel)?;
                }
                prefix.then(branches, fuel)?;
                Ok(prefix)
            }
            K::BitWritePointer {pointer,selections,value} => self.sequence(std::iter::once(pointer.as_ref()).chain(selections.iter().flat_map(nir::program::BitSelection::expressions)).chain(std::iter::once(value.as_ref())),n,fuel,depth),
            K::BitUpdate {selections,value,..} => self.sequence(selections.iter().flat_map(nir::program::BitSelection::expressions).chain(std::iter::once(value.as_ref())),n,fuel,depth),
            K::BitWindow {value, start, end} => self.sequence([value.as_ref(), start.as_ref(), end.as_ref()], n, fuel, depth),
            K::Match { value, arms } => self.match_control(value, arms, None, n, fuel, depth + 1),
            K::LetPattern {
                pattern,
                value,
                guard,
                otherwise,
            } => self.let_control(
                pattern,
                value,
                guard.as_deref(),
                otherwise.as_deref(),
                n,
                fuel,
                depth + 1,
            ),
            K::And(left, right) | K::Or(left, right) => {
                let mut prefix = self.control(left, n, fuel, depth + 1)?;
                let known = completed_bool(left, fuel, 0)?;
                let skip = matches!(expr.kind, K::And(..)) && known == Some(false)
                    || matches!(expr.kind, K::Or(..)) && known == Some(true);
                if skip || prefix.normal.is_none() {
                    return Ok(prefix);
                }
                let mut branch = self.control(right, n, fuel, depth + 1)?;
                if known.is_none() {
                    branch.merge(Paths::normal(vec![0; n]), fuel)?;
                }
                prefix.then(branch, fuel)?;
                Ok(prefix)
            }
            K::Return(value) => {
                let mut paths = self.sequence(value.as_deref(), n, fuel, depth)?;
                paths.divert(Exit::Return, fuel)?;
                Ok(paths)
            }
            K::EffectResume(value) => {
                let mut paths = self.control(value, n, fuel, depth + 1)?;
                paths.divert(Exit::Resume, fuel)?;
                Ok(paths)
            }
            K::CheckPredicate(value) => {
                let mut paths = self.control(value, n, fuel, depth + 1)?;
                if completed_bool(value, fuel, 0)? == Some(false) {
                    paths.divert(Exit::Abort, fuel)?;
                }
                Ok(paths)
            }
            K::Break(target) | K::Continue(target) => {
                let mut paths = Paths::normal(vec![0; n]);
                paths.divert(
                    if matches!(expr.kind, K::Break(_)) {
                        Exit::Break(*target)
                    } else {
                        Exit::Continue(*target)
                    },
                    fuel,
                )?;
                Ok(paths)
            }
            K::Loop { identity, body } => {
                let mut body = self.control(body, n, fuel, depth + 1)?;
                let mut back = body.normal.take();
                if let Some(uses) = body.exits.remove(&Exit::Continue(*identity)) {
                    if let Some(back) = &mut back {
                        join(back, &uses, fuel)?;
                    } else {
                        back = Some(uses);
                    }
                }
                let mut star = back.clone().unwrap_or_else(|| vec![0; n]);
                for count in &mut star {
                    if *count != 0 {
                        *count = 2;
                    }
                }
                let mut paths = Paths::default();
                if let Some(mut uses) = body.exits.remove(&Exit::Break(*identity)) {
                    add(&mut uses, &star, 1, fuel)?;
                    paths.normal = Some(uses);
                }
                for (target, mut uses) in body.exits {
                    add(&mut uses, &star, 1, fuel)?;
                    paths.exit(target, uses, fuel)?;
                }
                if back.is_some() {
                    paths.exit(Exit::Abort, star, fuel)?;
                }
                Ok(paths)
            }
            K::EffectComputation { body, .. } => self.control(body, n, fuel, depth + 1),
            K::EffectRun { computation, slots } => {
                self.mapped_control(computation, n, slots, fuel, depth + 1)
            }
            K::EffectRequest {
                allowed_index,
                arguments,
                ..
            } => {
                let mut paths = self.sequence(arguments, n, fuel, depth)?;
                if let Some(uses) = &mut paths.normal {
                    let slot = uses
                        .get_mut(*allowed_index)
                        .ok_or("effect usage request slot missing")?;
                    *slot = (*slot + 1).min(2);
                }
                Ok(paths)
            }
            K::Call {
                callee, arguments, ..
            } => {
                let mut paths = self.sequence(
                    std::iter::once(callee.as_ref()).chain(arguments.iter().map(|a| &a.value)),
                    n,
                    fuel,
                    depth,
                )?;
                if paths.normal.is_some() && !self.invoked(callee, fuel, depth + 1)?.is_empty() {
                    return Err("effect usage call lacks row mapping".into());
                }
                if expr.ty == Term::Primitive(nir::Primitive::Never) {
                    paths.divert(Exit::Abort, fuel)?;
                }
                Ok(paths)
            }
            K::Closure { captures, .. } => self.sequence(
                captures.iter().filter_map(|c| c.value.as_deref()),
                n,
                fuel,
                depth,
            ),
            // These operations evaluate their child expressions in source order.
            K::Let { value, .. }
            | K::Assign { value, .. }
            | K::Discard(value)
            | K::Not(value)
            | K::IntegerNegate(value)
            | K::FloatNegate(value)
            | K::AddressOf(value)
            | K::ReadPointer(value)
            | K::BitConvert(value)
            | K::Snapshot(value) | K::TakeBits(value) | K::SymbolName(value) | K::EnumName(value)
            | K::BitSlice { value, .. }
            | K::BitAssign { value, .. }
            | K::Field { value, .. }
            | K::AnonymousField { value, .. }
            | K::PointerField { pointer: value, .. }
            | K::PointerAnonymousField { pointer: value, .. } => {
                self.control(value, n, fuel, depth + 1)
            }
            K::Tuple(fields)
            | K::Record { fields, .. }
            | K::Variant { fields, .. }
            | K::AnonymousRecord { fields } => self.sequence(fields, n, fuel, depth),
            K::WritePointer {
                pointer: left,
                value: right,
            }
            | K::IntegerBinary { left, right, .. }
            | K::FloatBinary { left, right, .. }
            | K::ScalarEqual(left, right)
            | K::ScalarLessEqual(left, right)
            | K::ScalarLess(left, right)
            | K::ScalarGreater(left, right)
            | K::ScalarGreaterEqual(left, right)
            | K::BitEqual(left, right)
            | K::BitConcat(left, right) | K::BitIndex(left, right) => {
                self.sequence([left.as_ref(), right.as_ref()], n, fuel, depth)
            }
            _ => {
                let uses = self.raw(expr, n, fuel, depth + 1)?;
                self.conservative(expr, uses, fuel)
            }
        }?;
        if expr.ty == Term::Primitive(nir::Primitive::Never) {
            paths.divert(Exit::Abort, fuel)?;
        }
        Ok(paths)
    }
    fn handler_installation(
        &mut self,
        expr: &Expr,
        computation: &Expr,
        handlers: &[nir::program::EffectHandler],
        n: usize,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Paths, String> {
        step(fuel, n + handlers.len() + 1)?;
        let mut prefix = Paths::normal(vec![0; n]);
        for handler in handlers {
            if handler.abi != Abi::DirectPassing {
                continue;
            }
            if prefix.normal.is_none() {
                return Ok(prefix);
            }
            prefix.then(
                self.handler_paths(&handler.arms, n, handler.fallback, fuel, depth + 1)?,
                fuel,
            )?;
        }
        if prefix.normal.is_none() {
            return Ok(prefix);
        }
        let mut subject = self.control(computation, n + handlers.len(), fuel, depth + 1)?;
        let mut demand = vec![0; handlers.len()];
        for counts in subject.normal.iter().chain(subject.exits.values()) {
            join(&mut demand, &counts[..handlers.len()], fuel)?;
        }
        let mut requested = vec![0; n];
        let mut has_requested_handler = false;
        for (handler, upper) in handlers.iter().zip(demand) {
            if handler.abi != Abi::DirectPassing {
                if upper != 0 {
                    has_requested_handler = true;
                    let branch = self.handler_candidates(
                        &handler.arms,
                        n,
                        handler.fallback,
                        fuel,
                        depth + 1,
                    )?;
                    add(&mut requested, &branch, upper, fuel)?;
                }
                continue;
            }
            let Term::Function { result, .. } = &handler.signature else {
                return Err("effect usage handler signature missing".into());
            };
            let delivery = self.plan.delivery(&handler.operation, result, fuel)?;
            if let Some(report) = &mut self.report {
                if delivery == Delivery::Move && upper > 1 {
                    return Err(format!(
                        "affine effect slot may be consumed more than once at {}..{}: {}",
                        expr.source.start, expr.source.end, handler.operation
                    ));
                }
                for argument in &self.instance.arguments {
                    super::super::memory::bound_type(argument, fuel)?;
                }
                report.installations.push(Installation {
                    instance: self.instance.clone(),
                    source: expr.source,
                    operation: handler.operation.clone(),
                    upper,
                    delivery,
                });
            }
        }
        for counts in subject.normal.iter_mut().chain(subject.exits.values_mut()) {
            step(fuel, counts.len())?;
            counts.drain(..handlers.len());
        }
        if has_requested_handler {
            // Ordinary handlers can nonlocally exit the requesting computation.
            // Until request-site outcomes are substituted, retain their lexical
            // exits conservatively. Installation exits stay separate in prefix.
            let mut upper = subject.upper(n, fuel)?;
            add(&mut upper, &requested, 1, fuel)?;
            subject = self.conservative(expr, upper, fuel)?;
        }
        prefix.then(subject, fuel)?;
        Ok(prefix)
    }

    fn mapped_control(
        &mut self,
        expr: &Expr,
        n: usize,
        slots: &[usize],
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Paths, String> {
        step(fuel, n + 1)?;
        if depth >= 128 {
            return Err("effect usage mapped control depth exhausted".into());
        }
        match &expr.kind {
            K::Match { value, arms } => {
                self.match_control(value, arms, Some(slots), n, fuel, depth + 1)
            }
            K::Call {
                callee, arguments, ..
            } => {
                let mut paths = self.sequence(
                    std::iter::once(callee.as_ref()).chain(arguments.iter().map(|a| &a.value)),
                    n,
                    fuel,
                    depth,
                )?;
                if let Some(uses) = &mut paths.normal {
                    let invoked = self.invoked(callee, fuel, depth + 1)?;
                    if invoked.len() != slots.len() {
                        return Err("effect usage invocation row mismatch".into());
                    }
                    for (count, slot) in invoked.into_iter().zip(slots) {
                        let target = uses
                            .get_mut(*slot)
                            .ok_or("effect usage mapped slot missing")?;
                        *target = (*target + count).min(2);
                    }
                }
                Ok(paths)
            }
            K::EffectComputation { body, .. } => {
                let paths = self.control(body, slots.len(), fuel, depth + 1)?;
                let mut map = |counts: Uses| -> Result<Uses, String> {
                    step(fuel, n + counts.len())?;
                    let mut uses = vec![0; n];
                    for (count, slot) in counts.into_iter().zip(slots) {
                        let target = uses
                            .get_mut(*slot)
                            .ok_or("effect usage mapped slot missing")?;
                        *target = (*target + count).min(2);
                    }
                    Ok(uses)
                };
                Ok(Paths {
                    normal: paths.normal.map(&mut map).transpose()?,
                    exits: paths
                        .exits
                        .into_iter()
                        .map(|(exit, uses)| Ok((exit, map(uses)?)))
                        .collect::<Result<_, String>>()?,
                })
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let mut paths = self.sequence(statements, n, fuel, depth)?;
                if paths.normal.is_some() {
                    paths.then(
                        self.mapped_control(
                            result
                                .as_deref()
                                .ok_or("effect usage computation result missing")?,
                            n,
                            slots,
                            fuel,
                            depth + 1,
                        )?,
                        fuel,
                    )?;
                }
                Ok(paths)
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let mut prefix = self.control(condition, n, fuel, depth + 1)?;
                if prefix.normal.is_none() {
                    return Ok(prefix);
                }
                let mut branches = Paths::default();
                let known = completed_bool(condition, fuel, 0)?;
                if known != Some(false) {
                    branches.merge(
                        self.mapped_control(then_branch, n, slots, fuel, depth + 1)?,
                        fuel,
                    )?;
                }
                if known != Some(true) {
                    if let Some(branch) = else_branch {
                        branches.merge(
                            self.mapped_control(branch, n, slots, fuel, depth + 1)?,
                            fuel,
                        )?;
                    }
                }
                prefix.then(branches, fuel)?;
                Ok(prefix)
            }
            _ => Err("effect usage computation value is not lowered".into()),
        }
    }
}
