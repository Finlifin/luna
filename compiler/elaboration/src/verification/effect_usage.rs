//! Bounded request multiplicities, independent of type propositions. Counts are
//! upper bounds per invocation/installation, not ownership or cleanup proofs.
use iir::{
    Package,
    effect_values::{Delivery, Plan},
};
use nir::{
    Term,
    effects::Abi,
    program::{Expr, ExprKind as K, MatchArm, Pattern, SourceRange},
};
use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    fmt,
};
mod callables;
mod control;
mod returns;

type Uses = Vec<u8>; // 0, 1, or >= 2 / unknown repetition.
fn step(fuel: &mut usize, work: usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(work)
        .ok_or("effect usage budget exhausted")?;
    Ok(())
}
fn add(a: &mut Uses, b: &[u8], factor: u8, fuel: &mut usize) -> Result<(), String> {
    if a.len() != b.len() {
        return Err("effect usage row mismatch".into());
    }
    step(fuel, a.len())?;
    for (a, b) in a.iter_mut().zip(b) {
        *a = (*a + *b * factor).min(2);
    }
    Ok(())
}
fn join(a: &mut Uses, b: &[u8], fuel: &mut usize) -> Result<(), String> {
    if a.len() != b.len() {
        return Err("effect usage branch row mismatch".into());
    }
    step(fuel, a.len())?;
    for (a, b) in a.iter_mut().zip(b) {
        *a = (*a).max(*b);
    }
    Ok(())
}
fn width(signature: &Term) -> usize {
    match signature.callable_signature() {
        Term::Function { result, .. } => match result.as_ref() {
            Term::Computation { effects, .. } => effects.len(),
            _ => 0,
        },
        _ => 0,
    }
}
// This fact describes only normal completion. The caller must first account
// for all prefix effects and control exits; it cannot be used to skip evaluation.
fn completed_bool(expr: &Expr, fuel: &mut usize, depth: usize) -> Result<Option<bool>, String> {
    step(fuel, 1)?;
    if depth >= 128 {
        return Ok(None);
    }
    Ok(match &expr.kind {
        K::Literal(nir::program::Literal::Bool(value)) => Some(*value),
        K::Block {
            result: Some(result),
            ..
        }
        | K::CallPreparation {
            result: Some(result),
            ..
        } => completed_bool(result, fuel, depth + 1)?,
        K::Not(value) => completed_bool(value, fuel, depth + 1)?.map(|value| !value),
        K::And(left, right) => match completed_bool(left, fuel, depth + 1)? {
            Some(false) => Some(false),
            Some(true) => completed_bool(right, fuel, depth + 1)?,
            None => None,
        },
        K::Or(left, right) => match completed_bool(left, fuel, depth + 1)? {
            Some(true) => Some(true),
            Some(false) => completed_bool(right, fuel, depth + 1)?,
            None => None,
        },
        _ => None,
    })
}

#[derive(Clone, Debug)]
pub struct Installation {
    pub instance: iir::InstanceKey,
    pub source: SourceRange,
    pub operation: nir::effects::Operation,
    /// 0 = none, 1 = at most one, 2 = many/unbounded (not exactly two).
    pub upper: u8,
    pub delivery: Delivery,
}
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub installations: Vec<Installation>,
}
impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(effect-usage (installations")?;
        for item in &self.installations {
            write!(
                f,
                " (installation {} (source {} {}) {} (upper {}) (delivery {}))",
                item.instance,
                item.source.start,
                item.source.end,
                item.operation,
                match item.upper {
                    0 => "zero",
                    1 => "one",
                    _ => "many",
                },
                item.delivery
            )?;
        }
        f.write_str("))")
    }
}

/// Solve invocation summaries to a finite fixed point, then check affine slots.
/// Loops/unknown higher-order calls conservatively admit repeated requests.
/// Captured owner delivery, access dependencies and control cleanup are separate.
pub fn check(graph: &Package, plan: &Plan<'_>, fuel: &mut usize) -> Result<Report, String> {
    if !plan.belongs_to(graph) {
        return Err("effect usage plan belongs to another graph".into());
    }
    let mut instances = HashMap::new();
    let mut summaries = Vec::new();
    let mut aliases = Vec::new();
    for (index, function) in graph.functions.iter().enumerate() {
        for argument in &function.key.arguments {
            super::memory::bound_type(argument, fuel)?;
        }
        step(fuel, 1)?;
        if instances.insert(&function.key, index).is_some() {
            return Err("duplicate effect usage function instance".into());
        }
        let n = width(&function.function.signature);
        step(fuel, n + 1)?;
        summaries.push(vec![0; n]);
        aliases.push(callables::Aliases::prepare(&function.function, fuel)?);
    }
    step(fuel, graph.functions.len().saturating_mul(3))?;
    let mut readers = vec![BTreeSet::new(); graph.functions.len()];
    let mut queued = vec![true; graph.functions.len()];
    let mut pending: VecDeque<_> = (0..graph.functions.len()).collect();
    while let Some(index) = pending.pop_front() {
        step(fuel, 1)?;
        queued[index] = false;
        let function = &graph.functions[index];
        let mut dependencies = BTreeSet::new();
        let mut analyzer = Analyzer {
            functions: &graph.functions,
            all_aliases: &aliases,
            instances: &instances,
            plan,
            summaries: &summaries,
            instance: &function.key,
            report: None,
            aliases: &aliases[index],
            dependencies: Some(&mut dependencies),
        };
        let next = analyzer.expr(&function.function.body, summaries[index].len(), fuel, 0)?;
        // Register reads before publication, including a recursive self-read.
        // Retaining obsolete edges can only cause extra work, never a missed wakeup.
        for dependency in dependencies {
            step(fuel, 1)?;
            readers[dependency].insert(index);
        }
        let mut changed = false;
        for (old, next) in summaries[index].iter_mut().zip(next) {
            if next > *old {
                *old = next;
                changed = true;
            }
        }
        if changed {
            for &reader in &readers[index] {
                step(fuel, 1)?;
                if !queued[reader] {
                    queued[reader] = true;
                    pending.push_back(reader);
                }
            }
        }
    }
    let mut report = Report::default();
    for (index, function) in graph.functions.iter().enumerate() {
        let mut analyzer = Analyzer {
            functions: &graph.functions,
            all_aliases: &aliases,
            instances: &instances,
            plan,
            summaries: &summaries,
            instance: &function.key,
            report: Some(&mut report),
            dependencies: None,
            aliases: &aliases[index],
        };
        analyzer.expr(
            &function.function.body,
            width(&function.function.signature),
            fuel,
            0,
        )?;
        for value in function
            .function
            .preconditions
            .iter()
            .chain(&function.function.postconditions)
            .chain(function.function.defaults.iter().map(|(_, value)| value))
        {
            analyzer.expr(value, 0, fuel, 0)?;
        }
        // Closure construction only evaluates captures. Check separately the
        // installations belonging to its own invocation scope.
        iir::visit::expressions(&function.function, fuel, |expr, fuel| {
            if let K::Closure { body, .. } = &expr.kind {
                analyzer.expr(body, width(&expr.ty), fuel, 0)?;
            }
            Ok(())
        })?;
    }
    Ok(report)
}
struct Analyzer<'a, 'p, 's, 'r> {
    functions: &'a [iir::FunctionInstance],
    all_aliases: &'s [callables::Aliases<'a>],
    instances: &'s HashMap<&'a iir::InstanceKey, usize>,
    plan: &'p Plan<'a>,
    summaries: &'s [Uses],
    aliases: &'s callables::Aliases<'a>,
    instance: &'a iir::InstanceKey,
    report: Option<&'r mut Report>,
    dependencies: Option<&'r mut BTreeSet<usize>>,
}
impl Analyzer<'_, '_, '_, '_> {
    fn expr(
        &mut self,
        root: &Expr,
        n: usize,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Uses, String> {
        self.control(root, n, fuel, depth)?.upper(n, fuel)
    }
    fn raw(
        &mut self,
        root: &Expr,
        n: usize,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Uses, String> {
        step(fuel, n + 1)?;
        if depth >= 128 {
            return Err("effect usage depth exhausted".into());
        }
        let mut result = vec![0; n];
        iir::visit::expression_pruned(root, fuel, |expr, fuel| {
            let value = match &expr.kind {
                K::EffectRequest { .. }
                | K::EffectRun { .. }
                | K::EffectComputation { .. }
                | K::EffectHandle { .. }
                | K::Call { .. }
                | K::Closure { .. }
                | K::Block { .. }
                | K::CallPreparation { .. }
                | K::If { .. }
                | K::And(..)
                | K::Or(..)
                | K::Match { .. }
                | K::LetPattern { .. }
                | K::Loop { .. } => self.expr(expr, n, fuel, depth + 1)?,
                _ => return Ok(true),
            };
            add(&mut result, &value, 1, fuel)?;
            Ok(false)
        })?;
        Ok(result)
    }
    fn invoked(&mut self, callee: &Expr, fuel: &mut usize, depth: usize) -> Result<Uses, String> {
        step(fuel, 1)?;
        if depth >= 128 {
            return Err("effect usage callable depth exhausted".into());
        }
        let key = match &callee.kind {
            K::Call {
                callee: factory, ..
            } => {
                return self.returned_callable(factory, width(&callee.ty), fuel, depth + 1);
            }
            K::EffectRun { computation, .. } => {
                if let K::Call {
                    callee: factory, ..
                } = &computation.kind
                {
                    return self.returned_callable(factory, width(&callee.ty), fuel, depth + 1);
                }
                None
            }
            K::Field {
                value,
                declaration,
                index,
            } => {
                return self.field_callable(
                    value,
                    Some(*declaration),
                    *index,
                    width(&callee.ty),
                    fuel,
                    depth + 1,
                );
            }
            K::AnonymousField { value, index, .. } => {
                return self.field_callable(
                    value,
                    None,
                    *index,
                    width(&callee.ty),
                    fuel,
                    depth + 1,
                );
            }
            K::Local(binding) => {
                if let Some(uses) =
                    self.local_callable(*binding, width(&callee.ty), fuel, depth + 1)?
                {
                    return Ok(uses);
                }
                None
            }
            K::Match { value, arms } => {
                let mut uses = vec![0; width(&callee.ty)];
                for body in callables::selected_bodies(value, arms, fuel)? {
                    join(&mut uses, &self.invoked(body, fuel, depth + 1)?, fuel)?;
                }
                return Ok(uses);
            }
            // Callee preparation is already evaluated by control(). These
            // branches describe only the callable value produced on completion.
            K::Block {
                result: Some(result),
                ..
            }
            | K::CallPreparation {
                result: Some(result),
                ..
            } => {
                return self.invoked(result, fuel, depth + 1);
            }
            K::If {
                condition,
                then_branch,
                else_branch: Some(else_branch),
            } => {
                let mut uses = vec![0; width(&callee.ty)];
                let known = completed_bool(condition, fuel, 0)?;
                for (branch, reachable) in [
                    (then_branch, known != Some(false)),
                    (else_branch, known != Some(true)),
                ] {
                    if reachable && branch.ty != Term::Primitive(nir::Primitive::Never) {
                        join(&mut uses, &self.invoked(branch, fuel, depth + 1)?, fuel)?;
                    }
                }
                return Ok(uses);
            }
            K::Function(declaration) => Some(iir::InstanceKey {
                declaration: *declaration,
                arguments: vec![],
            }),
            K::FunctionInstance {
                declaration,
                arguments,
            } => {
                for argument in arguments {
                    super::memory::bound_type(argument, fuel)?;
                }
                Some(iir::InstanceKey {
                    declaration: *declaration,
                    arguments: arguments.clone(),
                })
            }
            K::Closure { body, .. } => return self.expr(body, width(&callee.ty), fuel, depth + 1),
            _ => None,
        };
        if let Some(key) = key {
            step(fuel, 1)?;
            let index = *self
                .instances
                .get(&key)
                .ok_or("effect usage callee instance missing")?;
            if let Some(dependencies) = &mut self.dependencies {
                step(fuel, 1)?;
                dependencies.insert(index);
            }
            step(fuel, self.summaries[index].len())?;
            return Ok(self.summaries[index].clone());
        }

        let n = width(&callee.ty);
        step(fuel, n + 1)?;
        Ok(vec![2; n])
    }
}
