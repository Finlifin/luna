//! A failed pattern retains its evaluated guards before the next candidate.
//! Pattern mismatch, branch success and nonlocal control are separate outcomes.
use super::*;
use crate::verification::pattern_facts::{irrefutable_shape, known_pattern};

#[derive(Default)]
struct Probe {
    yes: Option<Uses>,
    no: Option<Uses>,
    exits: BTreeMap<Exit, Uses>,
}
fn merge_count(target: &mut Option<Uses>, value: Uses, fuel: &mut usize) -> Result<(), String> {
    if let Some(target) = target {
        join(target, &value, fuel)?;
    } else {
        *target = Some(value);
    }
    Ok(())
}
impl Probe {
    fn choice(known: Option<bool>, n: usize, fuel: &mut usize) -> Result<Self, String> {
        step(fuel, n.saturating_mul(2).saturating_add(1))?;
        Ok(Self {
            yes: (known != Some(false)).then(|| vec![0; n]),
            no: (known != Some(true)).then(|| vec![0; n]),
            exits: BTreeMap::new(),
        })
    }
    fn prefix(&mut self, counts: &Uses, fuel: &mut usize) -> Result<(), String> {
        for uses in self
            .yes
            .iter_mut()
            .chain(&mut self.no)
            .chain(self.exits.values_mut())
        {
            add(uses, counts, 1, fuel)?;
        }
        Ok(())
    }
    fn merge_exits(&mut self, other: BTreeMap<Exit, Uses>, fuel: &mut usize) -> Result<(), String> {
        for (target, uses) in other {
            step(fuel, 1)?;
            if let Some(previous) = self.exits.get_mut(&target) {
                join(previous, &uses, fuel)?;
            } else {
                self.exits.insert(target, uses);
            }
        }
        Ok(())
    }
    fn next(&mut self, mut next: Self, fuel: &mut usize) -> Result<(), String> {
        let Some(prefix) = self.yes.take() else {
            return Ok(());
        };
        next.prefix(&prefix, fuel)?;
        self.yes = next.yes;
        if let Some(no) = next.no {
            merge_count(&mut self.no, no, fuel)?;
        }
        self.merge_exits(next.exits, fuel)
    }
}
// A later literal mismatch must not erase effects from an earlier guard. Only
// pure patterns may use the shared whole-pattern syntactic simplification.
fn pure(pattern: &Pattern, fuel: &mut usize, depth: usize) -> Result<bool, String> {
    step(fuel, 1)?;
    if depth >= 128 {
        return Err("effect usage pattern depth exhausted".into());
    }
    match pattern {
        Pattern::Then { .. } | Pattern::Guard { .. } => Ok(false),
        Pattern::Observe { inner, .. }
        | Pattern::Binding { inner, .. }
        | Pattern::Remap { pattern: inner, .. } => pure(inner, fuel, depth + 1),
        Pattern::Tuple(fields)
        | Pattern::TypeTuple(fields)
        | Pattern::Or(fields)
        | Pattern::NominalType { fields, .. }
        | Pattern::Record { fields, .. }
        | Pattern::AnonymousRecord { fields, .. }
        | Pattern::Variant { fields, .. }
        | Pattern::NormalForm {
            arguments: fields, ..
        } => {
            for field in fields {
                if !pure(field, fuel, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(true),
    }
}
impl Analyzer<'_, '_, '_, '_> {
    pub(in crate::verification::effect_usage) fn handler_candidates(
        &mut self,
        arms: &[MatchArm],
        n: usize,
        fallback: Option<usize>,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Uses, String> {
        self.handler_paths(arms, n, fallback, fuel, depth)?
            .upper(n, fuel)
    }

    pub(super) fn handler_paths(
        &mut self,
        arms: &[MatchArm],
        n: usize,
        fallback: Option<usize>,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Paths, String> {
        step(fuel, n + 1)?;
        let mut pending = Some(vec![0; n]);
        let mut output = Paths::default();
        for arm in arms {
            let Some(previous) = pending.take() else {
                break;
            };
            // Request arguments are unknown here; only irrefutable shapes and
            // evaluated guards can eliminate later candidates.
            let mut probe = self.probe(&arm.pattern, None, n, fuel, depth + 1)?;
            if let Some(guard) = &arm.guard {
                probe = self.guard_probe(probe, guard, n, fuel, depth + 1)?;
            }
            probe.prefix(&previous, fuel)?;
            if let Some(yes) = probe.yes {
                let mut branch = Paths::normal(yes);
                branch.then(self.control(&arm.body, n, fuel, depth + 1)?, fuel)?;
                output.merge(branch, fuel)?;
            }
            for (target, uses) in probe.exits {
                output.exit(target, uses, fuel)?;
            }
            pending = probe.no;
        }
        if let Some(mut uses) = pending {
            if let Some(slot) = fallback {
                let count = uses.get_mut(slot).ok_or("effect usage fallback missing")?;
                *count = (*count + 1).min(2);
                output.merge(Paths::normal(uses), fuel)?;
            } else {
                output.exit(Exit::Abort, uses, fuel)?;
            }
        }
        // resume supplies the current handler's result; it does not exit the
        // enclosing installation or execute the rest of the provider body.
        if let Some(uses) = output.exits.remove(&Exit::Resume) {
            output.merge(Paths::normal(uses), fuel)?;
        }
        Ok(output)
    }

    fn guard_probe(
        &mut self,
        mut probe: Probe,
        condition: &Expr,
        n: usize,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Probe, String> {
        let Some(prefix) = probe.yes.take() else {
            return Ok(probe);
        };
        let mut flow = Paths::normal(prefix);
        flow.then(self.control(condition, n, fuel, depth + 1)?, fuel)?;
        if let Some(uses) = flow.normal {
            let known = completed_bool(condition, fuel, 0)?;
            if known != Some(false) {
                step(fuel, uses.len())?;
                probe.yes = Some(uses.clone());
            }
            if known != Some(true) {
                merge_count(&mut probe.no, uses, fuel)?;
            }
        }
        probe.merge_exits(flow.exits, fuel)?;
        Ok(probe)
    }
    fn probe(
        &mut self,
        pattern: &Pattern,
        input: Option<&Expr>,
        n: usize,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Probe, String> {
        step(fuel, n + 1)?;
        if depth >= 128 {
            return Err("effect usage matching depth exhausted".into());
        }
        if pure(pattern, fuel, 0)? {
            let known = input
                .and_then(|value| known_pattern(pattern, value, 0))
                .or_else(|| irrefutable_shape(pattern, 0).then_some(true));
            return Probe::choice(known, n, fuel);
        }
        match pattern {
            Pattern::Observe { inner, .. }
            | Pattern::Binding { inner, .. }
            | Pattern::Remap { pattern: inner, .. } => self.probe(inner, input, n, fuel, depth + 1),
            Pattern::Guard { pattern, condition } => {
                let probe = self.probe(pattern, input, n, fuel, depth + 1)?;
                self.guard_probe(probe, condition, n, fuel, depth + 1)
            }
            Pattern::Then { first, value, next } => {
                let mut probe = self.probe(first, input, n, fuel, depth + 1)?;
                if let Some(prefix) = probe.yes.take() {
                    let mut flow = Paths::normal(prefix);
                    flow.then(self.control(value, n, fuel, depth + 1)?, fuel)?;
                    if let Some(prefix) = flow.normal {
                        probe.yes = Some(prefix);
                        let next = self.probe(next, Some(value), n, fuel, depth + 1)?;
                        probe.next(next, fuel)?;
                    }
                    probe.merge_exits(flow.exits, fuel)?;
                }
                Ok(probe)
            }
            Pattern::Or(alternatives) => {
                let mut result = Probe::default();
                let mut pending = Some(vec![0; n]);
                for alternative in alternatives {
                    let Some(prefix) = pending.take() else {
                        break;
                    };
                    let mut branch = self.probe(alternative, input, n, fuel, depth + 1)?;
                    branch.prefix(&prefix, fuel)?;
                    if let Some(yes) = branch.yes {
                        merge_count(&mut result.yes, yes, fuel)?;
                    }
                    pending = branch.no;
                    result.merge_exits(branch.exits, fuel)?;
                }
                result.no = pending;
                Ok(result)
            }
            Pattern::Tuple(fields)
            | Pattern::TypeTuple(fields)
            | Pattern::NominalType { fields, .. }
            | Pattern::Record { fields, .. }
            | Pattern::AnonymousRecord { fields, .. }
            | Pattern::Variant { fields, .. }
            | Pattern::NormalForm {
                arguments: fields, ..
            } => {
                let (shape, values): (Option<bool>, Option<&[Expr]>) = match (
                    pattern,
                    input.map(|e| &e.kind),
                ) {
                    (Pattern::Tuple(_), Some(K::Tuple(values)))
                    | (Pattern::TypeTuple(_), Some(K::TupleType(values))) => {
                        (Some(fields.len() == values.len()), Some(values))
                    }
                    (
                        Pattern::AnonymousRecord { names, .. },
                        Some(K::AnonymousRecord { fields: values }),
                    ) => {
                        let labels = input.is_some_and(|expr| matches!(&expr.ty, nir::Term::Record(types)
                                if names.len() == types.len() && names.iter().zip(types).all(|(name,(expected,_))|name == expected)));
                        (Some(labels && fields.len() == values.len()), Some(values))
                    }
                    (
                        Pattern::Record { declaration, .. },
                        Some(K::Record {
                            declaration: actual,
                            fields: values,
                        }),
                    ) => (
                        Some(declaration == actual && fields.len() == values.len()),
                        Some(values),
                    ),
                    (
                        Pattern::Variant {
                            declaration, index, ..
                        },
                        Some(K::Variant {
                            declaration: actual,
                            index: tag,
                            fields: values,
                        }),
                    ) => (
                        Some(declaration == actual && index == tag && fields.len() == values.len()),
                        Some(values),
                    ),
                    _ => (None, None),
                };
                let mut result = Probe::choice(shape, n, fuel)?;
                for (index, field) in fields.iter().enumerate() {
                    if result.yes.is_none() {
                        break;
                    }
                    let field =
                        self.probe(field, values.and_then(|v| v.get(index)), n, fuel, depth + 1)?;
                    result.next(field, fuel)?;
                }
                Ok(result)
            }
            _ => Probe::choice(None, n, fuel),
        }
    }
    pub(super) fn match_control(
        &mut self,
        value: &Expr,
        arms: &[MatchArm],
        slots: Option<&[usize]>,
        n: usize,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Paths, String> {
        step(fuel, n + 1)?;
        let mut prefix = self.control(value, n, fuel, depth + 1)?;
        if prefix.normal.is_none() {
            return Ok(prefix);
        }
        let mut pending = Some(vec![0; n]);
        let mut output = Paths::default();
        for arm in arms {
            let Some(previous) = pending.take() else {
                break;
            };
            let mut probe = self.probe(&arm.pattern, Some(value), n, fuel, depth + 1)?;
            if let Some(guard) = &arm.guard {
                probe = self.guard_probe(probe, guard, n, fuel, depth + 1)?;
            }
            probe.prefix(&previous, fuel)?;
            if let Some(yes) = probe.yes {
                let mut branch = Paths::normal(yes);
                branch.then(
                    match slots {
                        Some(slots) => self.mapped_control(&arm.body, n, slots, fuel, depth + 1)?,
                        None => self.control(&arm.body, n, fuel, depth + 1)?,
                    },
                    fuel,
                )?;
                output.merge(branch, fuel)?;
            }
            for (target, uses) in probe.exits {
                output.exit(target, uses, fuel)?;
            }
            pending = probe.no;
        }
        if let Some(uses) = pending {
            output.exit(Exit::Abort, uses, fuel)?;
        }
        prefix.then(output, fuel)?;
        Ok(prefix)
    }
    pub(super) fn let_control(
        &mut self,
        pattern: &Pattern,
        value: &Expr,
        guard: Option<&Expr>,
        otherwise: Option<&Expr>,
        n: usize,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Paths, String> {
        let mut prefix = self.control(value, n, fuel, depth + 1)?;
        if prefix.normal.is_none() {
            return Ok(prefix);
        }
        let mut probe = self.probe(pattern, Some(value), n, fuel, depth + 1)?;
        if let Some(guard) = guard {
            probe = self.guard_probe(probe, guard, n, fuel, depth + 1)?;
        }
        let mut paths = Paths {
            normal: probe.yes,
            exits: probe.exits,
        };
        if let Some(no) = probe.no {
            let mut failure = Paths::normal(no);
            if let Some(otherwise) = otherwise {
                failure.then(self.control(otherwise, n, fuel, depth + 1)?, fuel)?;
            } else {
                failure.divert(Exit::Abort, fuel)?;
            }
            paths.merge(failure, fuel)?;
        }
        prefix.then(paths, fuel)?;
        Ok(prefix)
    }
}
